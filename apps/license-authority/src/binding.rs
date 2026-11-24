//! Where a license may be used: its domains, host names and public addresses (checked when it
//! is issued, and on every request an installation makes with it).
//!
//! The address is the one thing a patched installation cannot choose: the authority sees where
//! the connection comes from. The host name and the domain are stated by the installation, so
//! they are signed with its key and kept with the request for the audit; they catch a license
//! moved to another machine or domain, and the entitlement carries them so the installation
//! checks them itself every time it works out what it may do.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::{Extensions, HeaderMap};
use builtpanel_http_kit::{AppError, FieldError};
use builtpanel_license::{Binding, BindingError, HostFacts, normalize_name};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{LaState, Settings};

/// The most entries a list may have.
pub const MAX_ENTRIES: usize = 50;

fn bad(field: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, "invalid", message)])
}

fn label_ok(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn name_ok(name: &str, min_labels: usize) -> bool {
    name.len() <= 253 && name.split('.').count() >= min_labels && name.split('.').all(label_ok)
}

/// Checks and normalises what an administrator typed: every list needs at least one entry.
///
/// # Errors
/// A validation error naming the list.
pub fn validate(
    domains: &[String],
    hostnames: &[String],
    ips: &[String],
) -> Result<Binding, AppError> {
    fn clean(
        field: &'static str,
        items: &[String],
        ok: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<String>, AppError> {
        if items.is_empty() || items.len() > MAX_ENTRIES {
            return Err(bad(field, "Between 1 and 50 entries."));
        }
        let mut out: Vec<String> = Vec::with_capacity(items.len());
        for item in items {
            let value = ok(item).ok_or_else(|| bad(field, "An entry is not valid."))?;
            if !out.contains(&value) {
                out.push(value);
            }
        }
        out.sort();
        Ok(out)
    }
    Ok(Binding {
        domains: clean("domains", domains, |d| {
            let d = normalize_name(d);
            name_ok(&d, 2).then_some(d)
        })?,
        hostnames: clean("hostnames", hostnames, |h| {
            let h = normalize_name(h);
            name_ok(&h, 1).then_some(h)
        })?,
        ips: clean("ips", ips, |i| {
            i.trim()
                .parse::<IpAddr>()
                .ok()
                .map(|ip| ip.to_canonical().to_string())
        })?,
    })
}

fn trusted(settings: &Settings, ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    settings
        .trusted_proxies
        .iter()
        .any(|p| p.to_canonical() == ip)
}

/// The address a request comes from. When the connection is from one of the authority's own
/// reverse proxies the address is the last one in `X-Forwarded-For` that is not a proxy;
/// otherwise the header is ignored, because anybody could write it.
#[must_use]
pub fn source_ip(
    settings: &Settings,
    extensions: &Extensions,
    headers: &HeaderMap,
) -> Option<IpAddr> {
    let peer = extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_canonical());
    if let Some(peer) = peer
        && !trusted(settings, peer)
    {
        return Some(peer);
    }
    // Behind a proxy of ours (or, in tests, without a socket): the forwarded chain, from the
    // proxy end, up to the first address that is not one of ours.
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| v.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
        .collect::<Vec<_>>();
    forwarded
        .iter()
        .rev()
        .find(|ip| !trusted(settings, **ip))
        .copied()
        .or(peer)
}

/// What a license is bound to, as the authority holds it.
#[derive(Debug, Clone, Default)]
pub struct Bound {
    /// Domains.
    pub domains: Vec<String>,
    /// Host names.
    pub hostnames: Vec<String>,
    /// Addresses.
    pub ips: Vec<String>,
}

impl Bound {
    /// The claims' form.
    #[must_use]
    pub fn binding(&self) -> Binding {
        Binding {
            domains: self.domains.clone(),
            hostnames: self.hostnames.clone(),
            ips: self.ips.clone(),
        }
    }
}

/// Refuses a request that does not come from where the license is bound, and notes it.
///
/// # Errors
/// `binding_mismatch` (403), or a validation error for a request without its host facts.
pub async fn enforce(
    state: &LaState,
    license_id: Uuid,
    installation_id: Uuid,
    bound: &Bound,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    domain: Option<&str>,
) -> Result<(), AppError> {
    let (Some(hostname), Some(domain)) = (hostname, domain) else {
        return Err(bad(
            "hostname",
            "The host name and the domain of the installation are needed.",
        ));
    };
    let binding = bound.binding();
    let verdict = binding
        .check_host(&HostFacts { hostname, domain })
        .and_then(|()| ip.map_or(Err(BindingError::Ip), |ip| binding.check_ip(&ip)));
    let Err(why) = verdict else {
        return Ok(());
    };
    note(
        &state.db,
        license_id,
        installation_id,
        why,
        ip,
        hostname,
        domain,
    )
    .await;
    let message = match why {
        BindingError::Unbound => {
            "This license is not bound to a domain, a host name and an address yet."
        }
        BindingError::Hostname => "This license is bound to another host name.",
        BindingError::Domain => "This license is bound to another domain.",
        BindingError::Ip => "This license is bound to another address.",
    };
    Err(AppError::forbidden("binding_mismatch", message))
}

/// Writes the refusal down: always in the audit log, and as an integrity event when the
/// installation is known. A failure to write it never changes the answer.
async fn note(
    db: &PgPool,
    license_id: Uuid,
    installation_id: Uuid,
    why: BindingError,
    ip: Option<IpAddr>,
    hostname: &str,
    domain: &str,
) {
    let detail = serde_json::json!({
        "license_id": license_id,
        "reason": format!("{why:?}").to_ascii_lowercase(),
        "ip": ip.map(|i| i.to_string()),
        "hostname": hostname.chars().take(253).collect::<String>(),
        "domain": domain.chars().take(253).collect::<String>(),
    });
    let _ = crate::audit(
        db,
        "installation",
        "license.binding_refused",
        Some(&license_id.to_string()),
        serde_json::json!({ "installation_id": installation_id, "detail": detail }),
    )
    .await;
    let _ = sqlx::query(
        "INSERT INTO integrity_events (installation_id, kind, detail)
         SELECT id, 'binding', $2 FROM installations WHERE id = $1",
    )
    .bind(installation_id)
    .bind(&detail)
    .execute(db)
    .await;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn settings() -> Settings {
        Settings {
            issuer: "la".into(),
            audience: "builtpanel".into(),
            entitlement_ttl: std::time::Duration::from_secs(1),
            session_ttl: std::time::Duration::from_secs(1),
            nonce_ttl: std::time::Duration::from_secs(1),
            key_dir: std::path::PathBuf::new(),
            trusted_proxies: vec!["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()],
            domain_proof: false,
            payment_webhook_secret: None,
            billing: crate::billing::Timing::default(),
        }
    }

    fn ext(peer: &str) -> Extensions {
        let mut e = Extensions::new();
        let socket = if peer.contains(':') {
            format!("[{peer}]:5000")
        } else {
            format!("{peer}:5000")
        };
        e.insert(ConnectInfo(socket.parse::<SocketAddr>().unwrap()));
        e
    }

    fn forwarded(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", value.parse().unwrap());
        h
    }

    #[test]
    fn the_address_is_the_connection_unless_a_proxy_of_ours_vouches_for_the_header() {
        let s = settings();
        let ip = |t: &str| Some(t.parse::<IpAddr>().unwrap());
        // Straight from a stranger: whatever the header says is ignored.
        assert_eq!(
            source_ip(&s, &ext("198.51.100.9"), &forwarded("203.0.113.7")),
            ip("198.51.100.9")
        );
        // Through our proxy: the last address that is not ours. A client that prepends its own
        // lie to the header only adds entries on the left.
        assert_eq!(
            source_ip(&s, &ext("127.0.0.1"), &forwarded("1.2.3.4, 203.0.113.7")),
            ip("203.0.113.7")
        );
        assert_eq!(
            source_ip(
                &s,
                &ext("::ffff:127.0.0.1"),
                &forwarded("203.0.113.7, 127.0.0.1")
            ),
            ip("203.0.113.7")
        );
        // Through our proxy without a header: the proxy itself.
        assert_eq!(
            source_ip(&s, &ext("127.0.0.1"), &HeaderMap::new()),
            ip("127.0.0.1")
        );
        // Rubbish in the header is skipped.
        assert_eq!(
            source_ip(&s, &ext("127.0.0.1"), &forwarded("not-an-ip, 203.0.113.7")),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn what_an_administrator_types_is_checked_and_normalised() {
        let b = validate(
            &["Panel.Example.COM.".into(), "panel.example.com".into()],
            &["Web1".into()],
            &["2001:0db8:0:0:0:0:0:7".into(), "::ffff:203.0.113.7".into()],
        )
        .unwrap();
        assert_eq!(b.domains, vec!["panel.example.com"]);
        assert_eq!(b.hostnames, vec!["web1"]);
        assert_eq!(b.ips, vec!["2001:db8::7", "203.0.113.7"]);
        for (d, h, i) in [
            (vec![], vec!["web1"], vec!["203.0.113.7"]),
            (vec!["example.com"], vec![], vec!["203.0.113.7"]),
            (vec!["example.com"], vec!["web1"], vec![]),
            (vec!["localhost"], vec!["web1"], vec!["203.0.113.7"]),
            (vec!["bad_domain.com"], vec!["web1"], vec!["203.0.113.7"]),
            (vec!["example.com"], vec!["-x"], vec!["203.0.113.7"]),
            (vec!["example.com"], vec!["web1"], vec!["203.0.113.0/24"]),
            (vec!["example.com"], vec!["web1"], vec!["nope"]),
        ] {
            let own = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
            assert!(
                validate(&own(&d), &own(&h), &own(&i)).is_err(),
                "{d:?} {h:?} {i:?}"
            );
        }
    }
}
