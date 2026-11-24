//! What an installation's stored license allows right now (T17.09, T17.10).
//!
//! [`evaluate`] is a pure function of what was stored, the trusted keys and the clock, so the
//! rules can be tested without a network or a database:
//!
//! * nothing stored: [`Mode::Unlicensed`];
//! * an entitlement that does not verify (forged, changed, for another installation, expired):
//!   [`Mode::Restricted`];
//! * the authority said no (revoked, suspended, expired): [`Mode::Restricted`] at once;
//! * a good entitlement last refreshed within the refresh period: [`Mode::Active`]; later,
//!   but within the offline grace (72 hours by default): [`Mode::Grace`] (works, warns); past
//!   the grace: [`Mode::Restricted`].

use builtpanel_license::{Claims, Document, Expect, PublicKeys};
use serde::{Deserialize, Serialize};

/// What is kept between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// The last entitlement the authority signed for this installation.
    pub entitlement: Option<Document>,
    /// When it was received, by this machine's clock (Unix seconds).
    pub last_refresh_ok_at: Option<i64>,
    /// When the last heartbeat got through.
    pub last_heartbeat_ok_at: Option<i64>,
    /// The authority's refusal (`license_revoked`, `license_suspended`, `license_expired`),
    /// until a refresh succeeds again.
    pub refused: Option<String>,
}

/// How the installation behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No license was ever entered.
    Unlicensed,
    /// A good license, recently confirmed.
    Active,
    /// A good license that the authority has not confirmed for a while; still works.
    Grace,
    /// Management is limited (sites keep running).
    Restricted,
}

/// The timings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// How often it is confirmed (seconds); later than this is [`Mode::Grace`].
    pub refresh_every: i64,
    /// How long it may go unconfirmed before [`Mode::Restricted`] (seconds).
    pub grace: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            refresh_every: 12 * 3600,
            grace: 72 * 3600,
        }
    }
}

/// The verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// How to behave.
    pub mode: Mode,
    /// Why, when it is not [`Mode::Active`] (a short code).
    pub reason: Option<String>,
    /// The claims, when the entitlement verified.
    pub claims: Option<Claims>,
    /// When the grace ends (Unix seconds), while it is running.
    pub grace_ends_at: Option<i64>,
}

/// A numeric limit of the license.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The license says nothing about it (treated as no limit by callers that choose to).
    Missing,
    /// No limit.
    Unlimited,
    /// At most this many.
    Max(i64),
}

impl Evaluation {
    /// Whether the license switches `feature` on.
    #[must_use]
    pub fn has_feature(&self, feature: &str) -> bool {
        self.claims
            .as_ref()
            .is_some_and(|c| c.features.iter().any(|f| f == feature))
    }

    /// A numeric limit.
    #[must_use]
    pub fn limit(&self, name: &str) -> Limit {
        match self.claims.as_ref().and_then(|c| c.limits.get(name)) {
            None => Limit::Missing,
            Some(n) if *n < 0 => Limit::Unlimited,
            Some(n) => Limit::Max(*n),
        }
    }

    /// Whether management is limited.
    #[must_use]
    pub fn restricted(&self) -> bool {
        self.mode == Mode::Restricted
    }
}

fn verdict(
    mode: Mode,
    reason: Option<&str>,
    claims: Option<Claims>,
    grace_ends_at: Option<i64>,
) -> Evaluation {
    Evaluation {
        mode,
        reason: reason.map(str::to_owned),
        claims,
        grace_ends_at,
    }
}

/// Decides what the stored license allows at `now`.
#[must_use]
pub fn evaluate(
    stored: &Stored,
    keys: &PublicKeys,
    expect: &Expect<'_>,
    policy: &Policy,
    now: i64,
) -> Evaluation {
    let Some(document) = &stored.entitlement else {
        return verdict(Mode::Unlicensed, Some("no_license"), None, None);
    };
    let claims = match document.verify(keys, expect, now) {
        Ok(c) => c,
        Err(builtpanel_license::DocumentError::Expired) => {
            return verdict(Mode::Restricted, Some("entitlement_expired"), None, None);
        }
        // Moved to another machine or another domain: the license is bound to where it was.
        Err(builtpanel_license::DocumentError::Binding(_)) => {
            return verdict(Mode::Restricted, Some("binding_mismatch"), None, None);
        }
        Err(_) => return verdict(Mode::Restricted, Some("entitlement_invalid"), None, None),
    };
    if let Some(code) = &stored.refused {
        return verdict(Mode::Restricted, Some(code), Some(claims), None);
    }
    let Some(last) = stored.last_refresh_ok_at else {
        return verdict(
            Mode::Restricted,
            Some("never_confirmed"),
            Some(claims),
            None,
        );
    };
    let age = now.saturating_sub(last);
    if age > policy.grace {
        verdict(
            Mode::Restricted,
            Some("offline_grace_exceeded"),
            Some(claims),
            None,
        )
    } else if age > policy.refresh_every {
        verdict(
            Mode::Grace,
            Some("not_confirmed"),
            Some(claims),
            Some(last + policy.grace),
        )
    } else {
        verdict(Mode::Active, None, Some(claims), None)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeMap;

    use builtpanel_license::{Claims, FileSigningKey, SCHEMA_VERSION, SigningKey as _};
    use uuid::Uuid;

    use super::*;

    const HOUR: i64 = 3600;

    struct Rig {
        key: FileSigningKey,
        keys: PublicKeys,
        inst: Uuid,
    }

    fn rig() -> Rig {
        let dir = std::env::temp_dir().join(format!("bp-state-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = FileSigningKey::generate_to_file(&dir.join("k"), "lic-1").unwrap();
        let keys = PublicKeys::new().with("lic-1", key.public_key()).unwrap();
        Rig {
            key,
            keys,
            inst: Uuid::from_u128(5),
        }
    }

    fn entitlement(r: &Rig, iat: i64, exp: i64) -> Document {
        Document::sign(
            &Claims {
                v: SCHEMA_VERSION,
                iss: "la".into(),
                aud: "builtpanel".into(),
                license_id: Uuid::from_u128(1),
                installation_id: r.inst,
                features: vec!["mail".into()],
                limits: BTreeMap::from([("max_nodes".into(), 2), ("max_tenants".into(), -1)]),
                bind: builtpanel_license::Binding {
                    domains: vec!["panel.example.com".into()],
                    hostnames: vec!["web1".into()],
                    ips: vec!["203.0.113.7".into()],
                },
                iat,
                exp,
                jti: Uuid::from_u128(9),
                kid: String::new(),
            },
            &r.key,
        )
        .unwrap()
    }

    fn at(r: &Rig, stored: &Stored, now: i64) -> Evaluation {
        let expect = Expect {
            issuer: "la",
            audience: "builtpanel",
            installation_id: r.inst,
            clock_skew: 60,
            host: builtpanel_license::HostFacts {
                hostname: "web1",
                domain: "panel.example.com",
            },
        };
        evaluate(stored, &r.keys, &expect, &Policy::default(), now)
    }

    #[test]
    fn a_license_moved_to_another_machine_or_domain_restricts_management() {
        let r = rig();
        let t0 = 1_000_000;
        let stored = Stored {
            entitlement: Some(entitlement(&r, t0, t0 + 200 * HOUR)),
            last_refresh_ok_at: Some(t0),
            ..Stored::default()
        };
        assert_eq!(at(&r, &stored, t0 + HOUR).mode, Mode::Active);
        for (hostname, domain) in [("web2", "panel.example.com"), ("web1", "other.example.org")] {
            let expect = Expect {
                issuer: "la",
                audience: "builtpanel",
                installation_id: r.inst,
                clock_skew: 60,
                host: builtpanel_license::HostFacts { hostname, domain },
            };
            let got = evaluate(&stored, &r.keys, &expect, &Policy::default(), t0 + HOUR);
            assert_eq!(
                (got.mode, got.reason.as_deref()),
                (Mode::Restricted, Some("binding_mismatch")),
                "{hostname} {domain}"
            );
        }
    }

    #[test]
    fn a_license_works_offline_for_72_hours_and_then_management_is_restricted() {
        let r = rig();
        let t0 = 1_000_000;
        let stored = Stored {
            entitlement: Some(entitlement(&r, t0, t0 + 200 * HOUR)),
            last_refresh_ok_at: Some(t0),
            ..Stored::default()
        };
        assert_eq!(at(&r, &stored, t0 + HOUR).mode, Mode::Active);
        assert_eq!(at(&r, &stored, t0 + 12 * HOUR).mode, Mode::Active);
        let grace = at(&r, &stored, t0 + 13 * HOUR);
        assert_eq!(
            (grace.mode, grace.grace_ends_at),
            (Mode::Grace, Some(t0 + 72 * HOUR))
        );
        assert!(!grace.restricted(), "still works while offline");
        assert_eq!(
            at(&r, &stored, t0 + 72 * HOUR).mode,
            Mode::Grace,
            "the 72nd hour is the last"
        );
        let past = at(&r, &stored, t0 + 72 * HOUR + 1);
        assert_eq!(
            (past.mode, past.reason.as_deref()),
            (Mode::Restricted, Some("offline_grace_exceeded"))
        );
        // Being confirmed again ends it.
        let confirmed = Stored {
            last_refresh_ok_at: Some(t0 + 80 * HOUR),
            ..stored
        };
        assert_eq!(at(&r, &confirmed, t0 + 81 * HOUR).mode, Mode::Active);
    }

    #[test]
    fn a_forged_or_changed_or_foreign_entitlement_gives_no_license() {
        let r = rig();
        let t0 = 1_000_000;
        let good = entitlement(&r, t0, t0 + 100 * HOUR);
        let mut changed = good.clone();
        changed.signature.replace_range(
            2..3,
            if &changed.signature[2..3] == "A" {
                "B"
            } else {
                "A"
            },
        );
        for entitlement in [changed, entitlement(&rig(), t0, t0 + 100 * HOUR)] {
            let stored = Stored {
                entitlement: Some(entitlement),
                last_refresh_ok_at: Some(t0),
                ..Stored::default()
            };
            let got = at(&r, &stored, t0 + HOUR);
            assert_eq!(
                (got.mode, got.reason.as_deref(), got.claims.is_none()),
                (Mode::Restricted, Some("entitlement_invalid"), true)
            );
        }
        // Another installation's entitlement.
        let other = Rig {
            inst: Uuid::from_u128(6),
            ..rig()
        };
        let stored = Stored {
            entitlement: Some(entitlement(&other, t0, t0 + 100 * HOUR)),
            last_refresh_ok_at: Some(t0),
            ..Stored::default()
        };
        assert_eq!(
            at(&r, &stored, t0 + HOUR).reason.as_deref(),
            Some("entitlement_invalid")
        );
        // Expired.
        let stored = Stored {
            entitlement: Some(good),
            last_refresh_ok_at: Some(t0 + 99 * HOUR),
            ..Stored::default()
        };
        assert_eq!(
            at(&r, &stored, t0 + 101 * HOUR).reason.as_deref(),
            Some("entitlement_expired")
        );
    }

    #[test]
    fn nothing_stored_a_refusal_and_an_unconfirmed_license_each_have_their_own_verdict() {
        let r = rig();
        let t0 = 1_000_000;
        assert_eq!(at(&r, &Stored::default(), t0).mode, Mode::Unlicensed);
        let mut stored = Stored {
            entitlement: Some(entitlement(&r, t0, t0 + 100 * HOUR)),
            ..Stored::default()
        };
        assert_eq!(
            at(&r, &stored, t0).reason.as_deref(),
            Some("never_confirmed")
        );
        stored.last_refresh_ok_at = Some(t0);
        stored.refused = Some("license_revoked".into());
        let got = at(&r, &stored, t0 + HOUR);
        assert_eq!(
            (got.mode, got.reason.as_deref()),
            (Mode::Restricted, Some("license_revoked"))
        );
        // The claims are there to read: a limit, a feature.
        stored.refused = None;
        let ok = at(&r, &stored, t0 + HOUR);
        assert!(ok.has_feature("mail") && !ok.has_feature("resellers"));
        assert_eq!(
            (
                ok.limit("max_nodes"),
                ok.limit("max_tenants"),
                ok.limit("nope")
            ),
            (Limit::Max(2), Limit::Unlimited, Limit::Missing)
        );
    }
}
