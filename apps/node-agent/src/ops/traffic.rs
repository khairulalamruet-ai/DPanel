//! `read_traffic`: counts an account's web requests from its web container's access log.
//!
//! The web server writes one line per request to standard output in the usual "combined"
//! form (`addr - user [time] "METHOD target PROTO" status bytes "referrer" "agent"`). The log
//! is read once and counted as it goes, nothing is kept but the counters, so a long log costs
//! time and not memory; the page table is capped. The client's address is not counted: behind
//! the container network it is not the visitor's.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use builtpanel_node_protocol::{
    AgentError, MAX_TRAFFIC_PATHS, Operation, Output, TrafficHour, TrafficPath,
};

use super::{
    sqlimport::{failed, invalid},
    tenant::{TenantContext, docker_error},
};

/// Most lines one count reads (the newest).
const MAX_LINES: u32 = 5_000_000;
/// Most different pages told apart; later new ones are counted in the totals only.
const MAX_PAGES: usize = 5_000;
/// Longest page path kept.
const MAX_PATH: usize = 200;
const HOUR_MS: i64 = 3_600_000;

/// One request, as far as the count needs it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub status: u16,
    pub bytes: u64,
    /// The path without the query string; `None` when the request line is not a path.
    pub path: Option<String>,
    pub bot: bool,
}

/// Reads one access-log line; `None` for a line that is not one.
pub(crate) fn parse(line: &str) -> Option<Hit> {
    let open = line.find("] \"")? + 3;
    let rest = &line[open..];
    let close = rest.find('"')?;
    let request = &rest[..close];
    let mut after = rest[close + 1..].split_whitespace();
    let status: u16 = after.next()?.parse().ok()?;
    if !(100..=599).contains(&status) {
        return None;
    }
    let bytes = after.next()?.parse().unwrap_or(0);
    let target = request.split_whitespace().nth(1);
    let path = target
        .map(|t| t.split(['?', '#']).next().unwrap_or(t))
        .filter(|p| p.starts_with('/'))
        .map(|p| p.chars().take(MAX_PATH).collect());
    let agent = line.rsplit('"').nth(1).unwrap_or_default().to_lowercase();
    let bot = ["bot", "crawl", "spider", "slurp"]
        .iter()
        .any(|w| agent.contains(w));
    Some(Hit {
        status,
        bytes,
        path,
        bot,
    })
}

/// The counters.
#[derive(Default)]
pub(crate) struct Count {
    requests: u64,
    bytes: u64,
    classes: [u64; 4],
    bots: u64,
    hours: BTreeMap<i64, (u64, u64)>,
    pages: HashMap<String, u64>,
}

impl Count {
    pub(crate) fn add(&mut self, at_ms: i64, hit: Hit) {
        self.requests += 1;
        self.bytes += hit.bytes;
        if let Some(class) = usize::from(hit.status / 100).checked_sub(2)
            && let Some(slot) = self.classes.get_mut(class)
        {
            *slot += 1;
        }
        self.bots += u64::from(hit.bot);
        let hour = self
            .hours
            .entry(at_ms.div_euclid(HOUR_MS) * HOUR_MS)
            .or_default();
        hour.0 += 1;
        hour.1 += hit.bytes;
        if let Some(path) = hit.path {
            if let Some(n) = self.pages.get_mut(&path) {
                *n += 1;
            } else if self.pages.len() < MAX_PAGES {
                self.pages.insert(path, 1);
            }
        }
    }

    pub(crate) fn into_output(self, top: usize, partial: bool) -> Output {
        let mut pages: Vec<(String, u64)> = self.pages.into_iter().collect();
        // Most first; equal counts in path order, so the answer does not change by chance.
        pages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        pages.truncate(top);
        Output::Traffic {
            requests: self.requests,
            bytes: self.bytes,
            status_2xx: self.classes[0],
            status_3xx: self.classes[1],
            status_4xx: self.classes[2],
            status_5xx: self.classes[3],
            bots: self.bots,
            hours: self
                .hours
                .into_iter()
                .map(|(at_ms, (requests, bytes))| TrafficHour {
                    at_ms,
                    requests,
                    bytes,
                })
                .collect(),
            top_paths: pages
                .into_iter()
                .map(|(path, requests)| TrafficPath { path, requests })
                .collect(),
            partial,
        }
    }
}

/// `read_traffic`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable), the account has no web container, or
/// Docker failing.
pub async fn read_traffic(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReadTraffic {
        username,
        since_ms,
        top,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let tenant =
        builtpanel_domain::TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if since_ms < 0 || !(1..=MAX_TRAFFIC_PATHS).contains(&top) {
        return Err(invalid("the time or the number of pages is out of range"));
    }
    let container =
        builtpanel_tenant_stack::web_container_name(&tenant).map_err(|e| failed(&e.to_string()))?;
    let mut count = Count::default();
    let mut lines = 0u32;
    context
        .docker
        .visit_logs(&container, since_ms, MAX_LINES, |at_ms, stderr, text| {
            if stderr {
                return;
            }
            lines = lines.saturating_add(1);
            if let Some(hit) = parse(text) {
                count.add(at_ms, hit);
            }
        })
        .await
        .map_err(|e| docker_error(&e))?;
    let top = usize::try_from(top).unwrap_or(usize::MAX);
    Ok(count.into_output(top, lines >= MAX_LINES))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const LINE: &str = r#"10.0.0.2 - - [22/Nov/2026:10:00:01 +0000] "GET /shop/item?id=4 HTTP/1.1" 200 5120 "https://example.com/" "Mozilla/5.0 (X11)""#;

    #[test]
    fn a_combined_line_is_read() {
        assert_eq!(
            parse(LINE),
            Some(Hit {
                status: 200,
                bytes: 5120,
                path: Some("/shop/item".into()),
                bot: false
            })
        );
    }

    #[test]
    fn robots_are_told_by_their_agent() {
        let line = LINE.replace(
            "Mozilla/5.0 (X11)",
            "Mozilla/5.0 (compatible; Googlebot/2.1)",
        );
        assert!(parse(&line).unwrap().bot);
        let line = LINE.replace("Mozilla/5.0 (X11)", "Slurp");
        assert!(parse(&line).unwrap().bot);
    }

    #[test]
    fn a_dash_for_bytes_is_nothing_and_odd_requests_are_still_counted() {
        let line = r#"1.2.3.4 - - [22/Nov/2026:10:00:01 +0000] "\x16\x03\x01" 400 - "-" "-""#;
        let hit = parse(line).unwrap();
        assert_eq!((hit.status, hit.bytes, hit.path), (400, 0, None));
        let line = r#"1.2.3.4 - - [22/Nov/2026:10:00:01 +0000] "OPTIONS * HTTP/1.1" 200 0 "-" "-""#;
        assert_eq!(parse(line).unwrap().path, None);
    }

    #[test]
    fn what_is_not_an_access_line_is_left_out() {
        for line in [
            "",
            "2026/11/22 10:00:00 [error] 1#1: *1 open() failed",
            r#"x [t] "GET / HTTP/1.1" 999 1 "-" "-""#,
            r#"x [t] "GET / HTTP/1.1" ok 1 "-" "-""#,
        ] {
            assert_eq!(parse(line), None, "{line}");
        }
    }

    #[test]
    fn requests_are_counted_by_class_hour_and_page() {
        let mut c = Count::default();
        let at = 5 * HOUR_MS + 17;
        for (status, path) in [
            (200, "/a"),
            (200, "/a"),
            (404, "/b"),
            (500, "/c"),
            (301, "/a"),
        ] {
            c.add(
                at,
                Hit {
                    status,
                    bytes: 10,
                    path: Some(path.into()),
                    bot: status == 404,
                },
            );
        }
        c.add(
            6 * HOUR_MS,
            Hit {
                status: 204,
                bytes: 0,
                path: None,
                bot: false,
            },
        );
        let Output::Traffic {
            requests,
            bytes,
            status_2xx,
            status_3xx,
            status_4xx,
            status_5xx,
            bots,
            hours,
            top_paths,
            partial,
        } = c.into_output(2, false)
        else {
            panic!("not a traffic answer");
        };
        assert_eq!((requests, bytes, bots), (6, 50, 1));
        assert_eq!(
            (status_2xx, status_3xx, status_4xx, status_5xx),
            (3, 1, 1, 1)
        );
        assert_eq!(
            hours,
            vec![
                TrafficHour {
                    at_ms: 5 * HOUR_MS,
                    requests: 5,
                    bytes: 50
                },
                TrafficHour {
                    at_ms: 6 * HOUR_MS,
                    requests: 1,
                    bytes: 0
                }
            ]
        );
        // Most first, ties by path, cut to `top`.
        assert_eq!(
            top_paths,
            vec![
                TrafficPath {
                    path: "/a".into(),
                    requests: 3
                },
                TrafficPath {
                    path: "/b".into(),
                    requests: 1
                }
            ]
        );
        assert!(!partial);
    }

    #[test]
    fn the_page_table_stops_growing() {
        let mut c = Count::default();
        for i in 0..(MAX_PAGES + 50) {
            c.add(
                0,
                Hit {
                    status: 200,
                    bytes: 1,
                    path: Some(format!("/p{i}")),
                    bot: false,
                },
            );
        }
        assert_eq!(c.pages.len(), MAX_PAGES);
        assert_eq!(c.requests, (MAX_PAGES + 50) as u64);
    }
}
