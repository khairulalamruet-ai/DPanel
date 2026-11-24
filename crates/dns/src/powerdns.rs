//! PowerDNS Authoritative, through its HTTP API.
//!
//! Zones are `Native` (the panel is the only writer; no replication between
//! servers is involved). Record sets are written whole: a (name, type) is
//! replaced by exactly the records the panel holds for it, and sets the panel
//! does not hold are deleted, apart from the SOA and the apex name servers,
//! which belong to the server.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use builtpanel_domain::{DnsRecord, DnsRecordType};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::{BoxFuture, DnsError, DnsProvider, ZoneSpec};

/// Time allowed for one call.
const TIMEOUT: Duration = Duration::from_secs(15);
/// Longest answer read, in bytes (a big zone is a few hundred kilobytes).
const MAX_BODY: u64 = 16 << 20;

/// Where PowerDNS is and how to talk to it.
#[derive(Clone)]
pub struct Config {
    /// Base address, `http://127.0.0.1:8053` (no path, no trailing slash).
    pub url: String,
    /// The API key.
    pub api_key: SecretString,
    /// PowerDNS's server id (`localhost`).
    pub server_id: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("server_id", &self.server_id)
            .finish_non_exhaustive()
    }
}

/// The PowerDNS provider. Cheap to clone.
#[derive(Clone)]
pub struct PowerDns {
    inner: Arc<Inner>,
}

struct Inner {
    base: String,
    key: SecretString,
    agent: ureq::Agent,
}

impl std::fmt::Debug for PowerDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PowerDns")
            .field("base", &self.inner.base)
            .finish_non_exhaustive()
    }
}

impl PowerDns {
    /// A client for the given server.
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            // The API key must go nowhere else.
            .max_redirects(0)
            .user_agent(format!("builtpanel/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            inner: Arc::new(Inner {
                base: format!(
                    "{}/api/v1/servers/{}",
                    config.url.trim_end_matches('/'),
                    config.server_id
                ),
                key: config.api_key.clone(),
                agent,
            }),
        }
    }

    /// Makes the apex name servers of an existing zone the wanted ones. With no
    /// name servers wanted the zone's own are left alone.
    async fn update_nameservers(&self, zone: &ZoneSpec, current: &Value) -> Result<(), DnsError> {
        if zone.nameservers.is_empty() {
            return Ok(());
        }
        let apex = format!("{}.", zone.name);
        let mut have: Vec<String> = current
            .get("rrsets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|s| {
                s.get("type").and_then(Value::as_str) == Some("NS")
                    && s.get("name").and_then(Value::as_str) == Some(apex.as_str())
            })
            .flat_map(|s| {
                s.get("records")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|r| r.get("content").and_then(Value::as_str))
            .map(str::to_ascii_lowercase)
            .collect();
        let mut want: Vec<String> = zone
            .nameservers
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();
        have.sort();
        want.sort();
        if have == want {
            return Ok(());
        }
        let rrset = json!({
            "name": apex,
            "type": "NS",
            "ttl": 3600,
            "changetype": "REPLACE",
            "records": zone.nameservers.iter().map(|n| json!({"content": n, "disabled": false})).collect::<Vec<_>>(),
        });
        let (status, body) = self
            .call(
                "PATCH",
                zone_path(&zone.name),
                Some(json!({ "rrsets": [rrset] })),
            )
            .await?;
        if status == 204 {
            Ok(())
        } else {
            Err(failure(status, &body))
        }
    }

    /// One call on the blocking pool: `(status, body)`.
    async fn call(
        &self,
        method: &'static str,
        path: String,
        body: Option<Value>,
    ) -> Result<(u16, Value), DnsError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.call(method, &path, body.as_ref()))
            .await
            .map_err(|_| DnsError::Unavailable("the call was cancelled".to_owned()))?
    }
}

impl Inner {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), DnsError> {
        let url = format!("{}{path}", self.base);
        let unavailable = |e: ureq::Error| DnsError::Unavailable(e.to_string());
        let mut response = match (method, body) {
            ("GET", _) => self
                .agent
                .get(&url)
                .header("X-API-Key", self.key.expose_secret())
                .call(),
            ("DELETE", _) => self
                .agent
                .delete(&url)
                .header("X-API-Key", self.key.expose_secret())
                .call(),
            ("POST", Some(b)) => self
                .agent
                .post(&url)
                .header("X-API-Key", self.key.expose_secret())
                .send_json(b),
            ("PATCH", Some(b)) => self
                .agent
                .patch(&url)
                .header("X-API-Key", self.key.expose_secret())
                .send_json(b),
            _ => return Err(DnsError::Rejected("unsupported call".to_owned())),
        }
        .map_err(unavailable)?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(unavailable)?;
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok((status, value))
    }
}

/// The server's own words for a refusal, shortened and without control characters.
fn reason(status: u16, body: &Value) -> String {
    let text = body
        .get("error")
        .and_then(Value::as_str)
        .map_or_else(|| format!("HTTP {status}"), str::to_owned);
    text.chars().filter(|c| !c.is_control()).take(300).collect()
}

/// Maps an unexpected status to the right error.
fn failure(status: u16, body: &Value) -> DnsError {
    if status >= 500 || status == 429 || status == 408 {
        DnsError::Unavailable(reason(status, body))
    } else {
        DnsError::Rejected(reason(status, body))
    }
}

// ----------------------------------------------------------- name and content

/// The absolute name of a record: `www` in `example.com` is `www.example.com.`.
fn absolute(zone: &str, relative: &str) -> String {
    if relative == "@" {
        format!("{zone}.")
    } else {
        format!("{relative}.{zone}.")
    }
}

/// The name relative to the zone, `@` for the zone itself.
fn relative(zone: &str, absolute: &str) -> Option<String> {
    let name = absolute.strip_suffix('.').unwrap_or(absolute);
    if name == zone {
        Some("@".to_owned())
    } else {
        name.strip_suffix(&format!(".{zone}")).map(str::to_owned)
    }
}

/// PowerDNS's text form of a TXT value: split in chunks of at most 255 bytes,
/// each in quotes. The record model allows printable ASCII without `"` and `\`
/// only (PowerDNS cannot store those through its API), so nothing needs escaping.
fn quote_txt(text: &str) -> String {
    if text.is_empty() {
        return "\"\"".to_owned();
    }
    text.as_bytes()
        .chunks(255)
        .map(|chunk| format!("\"{}\"", String::from_utf8_lossy(chunk)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads PowerDNS's TXT form back: the quoted chunks joined.
fn unquote_txt(content: &str) -> String {
    let mut out = String::new();
    let mut in_quote = false;
    let mut chars = content.chars();
    while let Some(c) = chars.next() {
        match (in_quote, c) {
            (_, '\\') if in_quote => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            (_, '"') => in_quote = !in_quote,
            (true, c) => out.push(c),
            (false, _) => {}
        }
    }
    out
}

fn wire_content(record: &DnsRecord) -> String {
    match record.rtype {
        DnsRecordType::Mx | DnsRecordType::Srv => {
            format!("{} {}", record.priority.unwrap_or(0), record.content)
        }
        DnsRecordType::Txt => quote_txt(&record.content),
        _ => record.content.clone(),
    }
}

fn from_wire(zone: &str, rrset: &Value) -> Option<Vec<DnsRecord>> {
    let name = relative(zone, rrset.get("name")?.as_str()?)?;
    let rtype = DnsRecordType::parse(rrset.get("type")?.as_str()?).ok()?;
    let ttl = u32::try_from(rrset.get("ttl")?.as_u64()?).ok()?;
    let mut out = Vec::new();
    for r in rrset.get("records")?.as_array()? {
        if r.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let content = r.get("content")?.as_str()?;
        let (priority, content) = match rtype {
            DnsRecordType::Mx | DnsRecordType::Srv => {
                let (p, rest) = content.split_once(' ')?;
                (Some(p.parse().ok()?), rest.to_owned())
            }
            DnsRecordType::Txt => (None, unquote_txt(content)),
            _ => (None, content.to_owned()),
        };
        out.push(DnsRecord {
            name: name.clone(),
            rtype,
            content,
            ttl,
            priority,
        });
    }
    Some(out)
}

/// The rrsets to send to make a zone hold `records`, given which sets exist now.
fn rrsets_to_send(zone: &str, records: &[DnsRecord], existing: &[Value]) -> Vec<Value> {
    // One set per (name, type); a set has one TTL, so the lowest wins.
    let mut wanted: BTreeMap<(String, &'static str), (u32, Vec<&DnsRecord>)> = BTreeMap::new();
    for r in records {
        let entry = wanted
            .entry((absolute(zone, &r.name), r.rtype.as_str()))
            .or_insert((r.ttl, Vec::new()));
        entry.0 = entry.0.min(r.ttl);
        entry.1.push(r);
    }
    let mut out: Vec<Value> = wanted
        .iter()
        .map(|((name, rtype), (ttl, rs))| {
            json!({
                "name": name,
                "type": rtype,
                "ttl": ttl,
                "changetype": "REPLACE",
                "records": rs.iter().map(|r| json!({"content": wire_content(r), "disabled": false})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let apex = format!("{zone}.");
    for set in existing {
        let (Some(name), Some(rtype)) = (
            set.get("name").and_then(Value::as_str),
            set.get("type").and_then(Value::as_str),
        ) else {
            continue;
        };
        // The server's own: the SOA, and the zone's name servers.
        if rtype == "SOA" || (rtype == "NS" && name == apex) {
            continue;
        }
        // Types this panel does not write (DNSKEY, RRSIG, ...) are left alone.
        if DnsRecordType::parse(rtype).is_err() {
            continue;
        }
        let known = wanted.keys().any(|(n, t)| n == name && *t == rtype);
        if !known {
            out.push(json!({"name": name, "type": rtype, "changetype": "DELETE"}));
        }
    }
    out
}

fn zone_path(zone: &str) -> String {
    format!("/zones/{zone}.")
}

impl DnsProvider for PowerDns {
    fn name(&self) -> &'static str {
        "powerdns"
    }

    fn ensure_zone<'a>(&'a self, zone: &'a ZoneSpec) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            let (status, body) = self.call("GET", zone_path(&zone.name), None).await?;
            match status {
                200 => return self.update_nameservers(zone, &body).await,
                404 => {}
                other => return Err(failure(other, &body)),
            }
            let create = json!({
                "name": format!("{}.", zone.name),
                "kind": "Native",
                "nameservers": zone.nameservers,
            });
            let (status, body) = self.call("POST", "/zones".to_owned(), Some(create)).await?;
            match status {
                201 => Ok(()),
                // Created by someone else between our look and our create.
                409 => Ok(()),
                other => Err(failure(other, &body)),
            }
        })
    }

    fn replace_records<'a>(
        &'a self,
        zone: &'a str,
        records: &'a [DnsRecord],
    ) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            let (status, body) = self.call("GET", zone_path(zone), None).await?;
            if status != 200 {
                return Err(failure(status, &body));
            }
            let existing = body
                .get("rrsets")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let rrsets = rrsets_to_send(zone, records, &existing);
            if rrsets.is_empty() {
                return Ok(());
            }
            let (status, body) = self
                .call("PATCH", zone_path(zone), Some(json!({ "rrsets": rrsets })))
                .await?;
            if status == 204 {
                Ok(())
            } else {
                Err(failure(status, &body))
            }
        })
    }

    fn list_records<'a>(
        &'a self,
        zone: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<DnsRecord>>, DnsError>> {
        Box::pin(async move {
            let (status, body) = self.call("GET", zone_path(zone), None).await?;
            match status {
                404 => return Ok(None),
                200 => {}
                other => return Err(failure(other, &body)),
            }
            let apex = format!("{zone}.");
            let mut out = Vec::new();
            for set in body
                .get("rrsets")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
            {
                let rtype = set.get("type").and_then(Value::as_str).unwrap_or_default();
                let name = set.get("name").and_then(Value::as_str).unwrap_or_default();
                if rtype == "SOA" || (rtype == "NS" && name == apex) {
                    continue;
                }
                if let Some(mut records) = from_wire(zone, set) {
                    out.append(&mut records);
                }
            }
            out.sort_by(|a, b| (&a.name, a.rtype, &a.content).cmp(&(&b.name, b.rtype, &b.content)));
            Ok(Some(out))
        })
    }

    fn delete_zone<'a>(&'a self, zone: &'a str) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            let (status, body) = self.call("DELETE", zone_path(zone), None).await?;
            match status {
                204 | 404 => Ok(()),
                other => Err(failure(other, &body)),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rec(name: &str, t: &str, content: &str, prio: Option<u32>) -> DnsRecord {
        DnsRecord::parse(name, t, content, Some(300), prio).unwrap()
    }

    #[test]
    fn names_go_to_the_server_absolute_and_come_back_relative() {
        assert_eq!(absolute("example.com", "@"), "example.com.");
        assert_eq!(absolute("example.com", "www"), "www.example.com.");
        assert_eq!(absolute("example.com", "*.shop"), "*.shop.example.com.");
        assert_eq!(relative("example.com", "example.com."), Some("@".into()));
        assert_eq!(
            relative("example.com", "www.example.com."),
            Some("www".into())
        );
        assert_eq!(
            relative("example.com", "a.b.example.com."),
            Some("a.b".into())
        );
        assert_eq!(relative("example.com", "badexample.com."), None);
        assert_eq!(relative("example.com", "other.org."), None);
    }

    #[test]
    fn text_is_quoted_and_split_for_the_server() {
        assert_eq!(quote_txt("v=spf1 a mx ~all"), "\"v=spf1 a mx ~all\"");
        let long = "x".repeat(600);
        let quoted = quote_txt(&long);
        assert_eq!(
            quoted.matches('"').count(),
            6,
            "three chunks of at most 255"
        );
        assert_eq!(unquote_txt(&quoted), long);
        for text in [
            "plain",
            "v=DKIM1; k=rsa; p=AB+/==",
            &"y".repeat(255),
            &"y".repeat(256),
        ] {
            assert_eq!(unquote_txt(&quote_txt(text)), text, "{text:?}");
        }
        // Text the server itself wrote may carry escapes; they are read back.
        assert_eq!(
            unquote_txt("\"a \\\\ b\" \"say \\\"x\\\"\""),
            "a \\ bsay \"x\""
        );
    }

    #[test]
    fn records_survive_the_trip_through_the_servers_format() {
        let zone = "example.com";
        let originals = vec![
            rec("@", "A", "192.0.2.1", None),
            rec("@", "MX", "mail.example.com.", Some(10)),
            rec("@", "TXT", "v=spf1 -all", None),
            rec("_sip._tcp", "SRV", "5 5060 sip.example.com.", Some(1)),
            rec("www", "CNAME", "example.com.", None),
        ];
        let sets = rrsets_to_send(zone, &originals, &[]);
        assert_eq!(sets.len(), 5);
        let mx = sets.iter().find(|s| s["type"] == "MX").unwrap();
        assert_eq!(mx["name"], "example.com.");
        assert_eq!(mx["records"][0]["content"], "10 mail.example.com.");
        let back: Vec<DnsRecord> = sets
            .iter()
            .flat_map(|s| from_wire(zone, s).unwrap())
            .collect();
        for original in &originals {
            assert!(
                back.contains(original),
                "{original:?} missing from {back:?}"
            );
        }
    }

    #[test]
    fn sets_that_are_not_wanted_are_deleted_but_the_servers_own_are_not() {
        let zone = "example.com";
        let existing = vec![
            json!({"name": "example.com.", "type": "SOA", "ttl": 3600, "records": []}),
            json!({"name": "example.com.", "type": "NS", "ttl": 3600, "records": []}),
            json!({"name": "old.example.com.", "type": "A", "ttl": 300, "records": []}),
            json!({"name": "www.example.com.", "type": "A", "ttl": 300, "records": []}),
            json!({"name": "sub.example.com.", "type": "NS", "ttl": 300, "records": []}),
            json!({"name": "example.com.", "type": "DNSKEY", "ttl": 300, "records": []}),
        ];
        let wanted = vec![rec("www", "A", "192.0.2.1", None)];
        let sets = rrsets_to_send(zone, &wanted, &existing);
        let deleted: Vec<String> = sets
            .iter()
            .filter(|s| s["changetype"] == "DELETE")
            .map(|s| {
                format!(
                    "{} {}",
                    s["name"].as_str().unwrap(),
                    s["type"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(deleted, ["old.example.com. A", "sub.example.com. NS"]);
        let replaced: Vec<_> = sets
            .iter()
            .filter(|s| s["changetype"] == "REPLACE")
            .collect();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0]["ttl"], 300);
        // Two records of one name and type are one set, with the lowest TTL.
        let two = vec![
            DnsRecord::parse("@", "A", "192.0.2.1", Some(600), None).unwrap(),
            DnsRecord::parse("@", "A", "192.0.2.2", Some(120), None).unwrap(),
        ];
        let sets = rrsets_to_send(zone, &two, &[]);
        assert_eq!(
            (
                sets.len(),
                sets[0]["ttl"].clone(),
                sets[0]["records"].as_array().unwrap().len()
            ),
            (1, json!(120), 2)
        );
    }

    #[test]
    fn failures_are_told_apart_by_whether_trying_again_can_help() {
        let body = json!({"error": "Domain 'x' does not exist"});
        for status in [500, 502, 503, 429, 408] {
            assert!(failure(status, &body).is_retryable(), "{status}");
        }
        for status in [400, 401, 403, 404, 409, 422] {
            assert!(!failure(status, &body).is_retryable(), "{status}");
        }
        assert!(failure(422, &body).to_string().contains("does not exist"));
        assert_eq!(reason(422, &Value::Null), "HTTP 422");
        let noisy = json!({"error": format!("bad\n{}", "x".repeat(1000))});
        let text = reason(422, &noisy);
        assert!(text.chars().count() <= 300 && !text.contains('\n'));
    }

    #[test]
    fn the_api_key_is_never_shown() {
        let config = Config {
            url: "http://127.0.0.1:8053".into(),
            api_key: SecretString::from("super-secret-key".to_owned()),
            server_id: "localhost".into(),
        };
        let shown = format!("{config:?} {:?}", PowerDns::new(&config));
        assert!(!shown.contains("super-secret-key"), "{shown}");
    }
}
