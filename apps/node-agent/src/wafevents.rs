//! `read_waf_events`: what the web application firewall stopped, from its audit log.
//!
//! The firewall (Coraza in the node's Caddy, see `deploy/caddy/coraza.conf`) appends one JSON
//! document per refused request to a file, `BUILTPANEL_AGENT_WAF_LOG` (default
//! `/var/log/caddy/waf-audit.log`). Only the end of the file is read, a line at a time, and only
//! the newest events of the asked names are kept: a long log costs time, not memory. The
//! visitor's address is not passed on, and neither is anything of the request but its method,
//! name and address.

use std::{collections::VecDeque, io::SeekFrom, path::PathBuf, sync::Arc};

use builtpanel_node_protocol::{
    AgentError, ErrorCode, MAX_WAF_EVENTS, MAX_WAF_HOSTS, Operation, Output, WafEvent,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::web::WebContext;

/// How much of the end of the log one read goes through.
const TAIL_BYTES: u64 = 8 * 1024 * 1024;
/// The longest line read; longer ones (a request body logged whole) are skipped.
const MAX_LINE: usize = 256 * 1024;
/// Longest text kept of an address or a message.
const MAX_TEXT: usize = 200;
/// The rules that add the scores up and block: they say nothing about the attack itself.
const SCORE_RULES: [u32; 3] = [949_110, 959_100, 980_130];
/// Where the log is when nothing says otherwise.
pub const DEFAULT_LOG: &str = "/var/log/caddy/waf-audit.log";

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

/// The first value of a header, whatever the case of its name.
fn header(headers: &Value, name: &str) -> Option<String> {
    headers.as_object()?.iter().find_map(|(k, v)| {
        k.eq_ignore_ascii_case(name).then(|| {
            v.as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .or_else(|| v.as_str())
                .unwrap_or_default()
                .to_owned()
        })
    })
}

/// Reads one audit-log line; `None` for a line that is not one.
pub(crate) fn parse(line: &str) -> Option<WafEvent> {
    let doc: Value = serde_json::from_str(line.trim()).ok()?;
    let transaction = doc.get("transaction")?;
    let request = transaction.get("request")?;
    let rule_of = |m: &Value| {
        m.get("data")
            .and_then(|d| d.get("id"))
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .unwrap_or(0)
    };
    let messages: &[Value] = doc
        .get("messages")
        .or_else(|| transaction.get("messages"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    // The rule that names the attack, not the one that adds the scores up.
    let message = messages
        .iter()
        .find(|m| !SCORE_RULES.contains(&rule_of(m)))
        .or_else(|| messages.first());
    let text = message
        .and_then(|m| {
            m.get("data")
                .and_then(|d| d.get("msg"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| m.get("message").and_then(Value::as_str))
        })
        .unwrap_or_default();
    let host = request
        .get("headers")
        .and_then(|h| header(h, "host"))
        .or_else(|| {
            transaction
                .get("server_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    // Without the port, in lower case, as the sites are named.
    let host = host
        .rsplit_once(':')
        .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host.as_str(), |(h, _)| h)
        .to_ascii_lowercase();
    // Nanoseconds since the epoch (older builds: seconds).
    let at_ms = transaction
        .get("unix_timestamp")
        .and_then(Value::as_i64)
        .map(|t| {
            if t > 100_000_000_000_000 {
                t / 1_000_000
            } else if t > 100_000_000_000 {
                t
            } else {
                t * 1000
            }
        })
        .unwrap_or(0);
    let status = transaction
        .get("response")
        .and_then(|r| r.get("status"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let blocked = transaction
        .get("is_interrupted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || status == 403;
    Some(WafEvent {
        at_ms,
        method: cut(request.get("method").and_then(Value::as_str).unwrap_or("-")),
        host: cut(&host),
        uri: cut(request.get("uri").and_then(Value::as_str).unwrap_or("/")),
        rule_id: message.map_or(0, rule_of),
        message: cut(text),
        blocked,
    })
}

/// Filters and orders: the newest `limit` events of these names since a time, newest first.
pub(crate) fn newest(
    lines: impl Iterator<Item = String>,
    hosts: &std::collections::BTreeSet<String>,
    since_ms: i64,
    limit: usize,
) -> (Vec<WafEvent>, bool) {
    let mut kept: VecDeque<WafEvent> = VecDeque::new();
    let mut dropped = false;
    for line in lines {
        let Some(event) = parse(&line) else { continue };
        if event.at_ms < since_ms || !hosts.contains(&event.host) {
            continue;
        }
        if kept.len() == limit {
            kept.pop_front();
            dropped = true;
        }
        kept.push_back(event);
    }
    (kept.into_iter().rev().collect(), dropped)
}

/// `read_waf_events`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable), or the log cannot be read.
pub async fn read_waf_events(
    context: Arc<WebContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReadWafEvents {
        hosts,
        since_ms,
        limit,
        ..
    } = op
    else {
        return Err(crate::ops::wrong_operation());
    };
    if since_ms < 0 || !(1..=MAX_WAF_EVENTS).contains(&limit) || hosts.len() > MAX_WAF_HOSTS {
        return Err(AgentError::new(
            ErrorCode::InvalidArguments,
            "the time, the number of events or of names is out of range".to_owned(),
        ));
    }
    // A web server without the firewall has no log to read.
    if !context.has_waf() {
        return Ok(Output::WafEvents {
            available: false,
            events: Vec::new(),
            partial: false,
        });
    }
    let path = std::env::var_os("BUILTPANEL_AGENT_WAF_LOG")
        .map_or_else(|| PathBuf::from(DEFAULT_LOG), PathBuf::from);
    read_file(&path, hosts, since_ms, limit).await
}

/// Reads the end of the log at `path`.
async fn read_file(
    path: &std::path::Path,
    hosts: Vec<String>,
    since_ms: i64,
    limit: u32,
) -> Result<Output, AgentError> {
    let unavailable = || Output::WafEvents {
        available: false,
        events: Vec::new(),
        partial: false,
    };
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        // No request has been refused yet, or the log is not mounted here.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(unavailable()),
        Err(e) => {
            return Err(AgentError::new(
                ErrorCode::Internal,
                format!("the firewall's log cannot be opened: {}", e.kind()),
            ));
        }
    };
    let size = file
        .metadata()
        .await
        .map_err(|e| {
            AgentError::new(
                ErrorCode::Internal,
                format!("the log cannot be read: {}", e.kind()),
            )
        })?
        .len();
    let start = size.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).await.map_err(|e| {
        AgentError::new(
            ErrorCode::Internal,
            format!("the log cannot be read: {}", e.kind()),
        )
    })?;
    let mut bytes = Vec::with_capacity(usize::try_from(size - start).unwrap_or(0));
    file.take(TAIL_BYTES)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| {
            AgentError::new(
                ErrorCode::Internal,
                format!("the log cannot be read: {}", e.kind()),
            )
        })?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    // The first line of a cut read is a piece of one.
    if start > 0 {
        lines.next();
    }
    let hosts: std::collections::BTreeSet<String> = hosts.into_iter().collect();
    let (events, dropped) = newest(
        lines.filter(|l| l.len() <= MAX_LINE).map(str::to_owned),
        &hosts,
        since_ms,
        usize::try_from(limit).unwrap_or(usize::MAX),
    );
    Ok(Output::WafEvents {
        available: true,
        events,
        partial: dropped || start > 0,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// What Coraza writes (measured on the build box with the CRS 4.17.1 image), shortened.
    fn line(host: &str, ns: i64, interrupted: bool, uri: &str) -> String {
        serde_json::json!({
            "transaction": {
                "timestamp": "2026/11/22 11:03:15", "unix_timestamp": ns, "id": "COGGCcKujtkLzJgj",
                "client_ip": "10.0.2.2", "client_port": 0, "server_id": host,
                "request": {"method": "GET", "protocol": "HTTP/1.1", "uri": uri,
                    "headers": {"accept": ["*/*"], "host": [host], "user-agent": ["curl/8.5.0"]}},
                "response": {"protocol": "", "status": 0, "headers": {}, "body": ""},
                "highest_severity": "", "is_interrupted": interrupted
            },
            "messages": [
                {"actionset": "OWASP_CRS/4.17.1", "message": "SQL Injection Attack Detected via libinjection",
                 "data": {"id": 942100, "msg": "SQL Injection Attack Detected via libinjection", "data": "Matched Data: s&sos found within ARGS:id"}},
                {"actionset": "OWASP_CRS/4.17.1", "message": "Inbound Anomaly Score Exceeded (Total Score: 5)",
                 "data": {"id": 949110, "msg": "Inbound Anomaly Score Exceeded (Total Score: 5)"}}
            ]
        })
        .to_string()
    }

    const NS: i64 = 1_795_345_395_351_521_221;

    #[test]
    fn a_refused_request_names_the_attack_and_not_the_score_rule() {
        let e = parse(&line("Shop.Example.com:443", NS, true, "/?id=1%27")).unwrap();
        assert_eq!(e.at_ms, 1_795_345_395_351);
        assert_eq!(
            (e.method.as_str(), e.host.as_str(), e.rule_id, e.blocked),
            ("GET", "shop.example.com", 942_100, true)
        );
        assert_eq!(e.message, "SQL Injection Attack Detected via libinjection");
        assert_eq!(e.uri, "/?id=1%27");
    }

    #[test]
    fn a_line_without_the_flag_still_counts_when_the_answer_was_403() {
        let mut doc: Value = serde_json::from_str(&line("a.com", NS, false, "/")).unwrap();
        assert!(!parse(&doc.to_string()).unwrap().blocked);
        doc["transaction"]["response"]["status"] = 403.into();
        assert!(parse(&doc.to_string()).unwrap().blocked);
    }

    #[test]
    fn what_is_not_an_audit_line_is_left_out() {
        for text in [
            "",
            "2026/11/22 [error] x",
            "{}",
            r#"{"transaction": {}}"#,
            r#"{"other": 1}"#,
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn the_newest_events_of_the_asked_names_come_first_and_the_rest_is_left_out() {
        let hosts: std::collections::BTreeSet<String> = ["a.com".to_owned()].into();
        let lines = vec![
            line("a.com", NS, true, "/1"),
            line("b.com", NS + 1_000_000_000, true, "/other-site"),
            line("a.com", NS + 2_000_000_000, true, "/2"),
            "not json".to_owned(),
            line("a.com", NS + 3_000_000_000, true, "/3"),
        ];
        let (events, dropped) = newest(lines.clone().into_iter(), &hosts, 0, 10);
        assert_eq!(
            events.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(),
            ["/3", "/2", "/1"]
        );
        assert!(!dropped);
        // Cut to the newest two.
        let (events, dropped) = newest(lines.clone().into_iter(), &hosts, 0, 2);
        assert_eq!(
            events.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(),
            ["/3", "/2"]
        );
        assert!(dropped);
        // Since a time.
        let (events, _) = newest(lines.into_iter(), &hosts, 1_795_345_397_000, 10);
        assert_eq!(
            events.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(),
            ["/3", "/2"]
        );
    }

    #[tokio::test]
    async fn the_file_is_read_from_its_end_and_a_missing_one_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("bp-wafevents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("waf-audit.log");
        let _ = std::fs::remove_file(&file);
        let none = read_file(&file, vec!["a.com".into()], 0, 10).await.unwrap();
        assert!(matches!(
            none,
            Output::WafEvents {
                available: false,
                ..
            }
        ));

        let text = [
            line("a.com", NS, true, "/old"),
            "a line cut off in the mi".to_owned(),
            line("b.com", NS + 1_000_000_000, true, "/not-mine"),
            line("a.com", NS + 2_000_000_000, true, "/new"),
        ]
        .join("\n");
        std::fs::write(&file, text + "\n").unwrap();
        let got = read_file(&file, vec!["a.com".into()], 0, 10).await.unwrap();
        let Output::WafEvents {
            available,
            events,
            partial,
        } = got
        else {
            panic!("not an answer of events");
        };
        assert!(available && !partial);
        assert_eq!(
            events.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(),
            ["/new", "/old"]
        );
        // Only the newest one.
        let Output::WafEvents {
            events, partial, ..
        } = read_file(&file, vec!["a.com".into()], 0, 1).await.unwrap()
        else {
            panic!("not an answer of events");
        };
        assert_eq!((events.len(), partial), (1, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn texts_are_cut() {
        let long = "a".repeat(500);
        let e = parse(&line("a.com", NS, true, &long)).unwrap();
        assert_eq!(e.uri.chars().count(), MAX_TEXT);
    }
}
