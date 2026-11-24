//! The entitlement a license authority issues (T17.04, T17.09): claims, signed, and checked
//! by the Built Panel installation it was issued to.
//!
//! A [`Document`] is the claims in canonical JSON plus the key id and the Ed25519 signature
//! over a domain-separated form of those bytes. [`Document::verify`] refuses an unknown key,
//! a signature that does not match, a document for another issuer, audience or installation,
//! one that is not yet valid or has expired, and anything that is not exactly what the
//! authority wrote (a changed byte, an added claim).

use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    canonical::{CanonicalError, canonical_json},
    keys::{PublicKeys, SigningKey},
};

/// The schema version of the claims.
pub const SCHEMA_VERSION: u32 = 1;

/// What a signature covers is prefixed with this, so a signature made for one purpose is
/// never valid for another (entitlements, release manifests and so on use their own).
pub const ENTITLEMENT_DOMAIN: &[u8] = b"builtpanel-entitlement-v1\n";

/// What a license is bound to. All three lists are part of the signed claims, so they cannot be
/// changed by whoever holds the entitlement. A host name or domain is written in lower case, a
/// domain without a trailing dot, an address in its usual text form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// The domains the panel is reached at (a sub-domain of a listed domain is allowed).
    pub domains: Vec<String>,
    /// The machine's host names.
    pub hostnames: Vec<String>,
    /// The public addresses the machine is seen from (checked by the authority).
    pub ips: Vec<String>,
}

/// What a host says about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostFacts<'a> {
    /// The machine's host name.
    pub hostname: &'a str,
    /// The domain the panel is reached at.
    pub domain: &'a str,
}

/// Why a host does not fit a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    /// The license is not bound to anything, so it cannot be used.
    #[error("the license is not bound to a domain, a host name and an address")]
    Unbound,
    /// The host name is not one the license is bound to.
    #[error("this host name is not the one the license is bound to")]
    Hostname,
    /// The domain is not one the license is bound to.
    #[error("this domain is not the one the license is bound to")]
    Domain,
    /// The address is not one the license is bound to.
    #[error("this address is not the one the license is bound to")]
    Ip,
}

/// Lower case, no trailing dot.
#[must_use]
pub fn normalize_name(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl Binding {
    /// Whether all three lists name something (a license that is bound to nothing is not used).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.domains.is_empty() && !self.hostnames.is_empty() && !self.ips.is_empty()
    }

    /// Whether `hostname` and `domain` fit (the address is the authority's to check: the
    /// installation cannot see how it is seen from outside).
    ///
    /// # Errors
    /// [`BindingError`].
    pub fn check_host(&self, host: &HostFacts<'_>) -> Result<(), BindingError> {
        if !self.is_complete() {
            return Err(BindingError::Unbound);
        }
        let hostname = normalize_name(host.hostname);
        if !self.hostnames.contains(&hostname) {
            return Err(BindingError::Hostname);
        }
        let domain = normalize_name(host.domain);
        let fits = self
            .domains
            .iter()
            .any(|d| domain == *d || domain.ends_with(&format!(".{d}")));
        if fits {
            Ok(())
        } else {
            Err(BindingError::Domain)
        }
    }

    /// Whether the address the authority sees fits.
    ///
    /// # Errors
    /// [`BindingError`].
    pub fn check_ip(&self, ip: &std::net::IpAddr) -> Result<(), BindingError> {
        if !self.is_complete() {
            return Err(BindingError::Unbound);
        }
        // Compared as addresses, not as text (`::1` and `0:0:0:0:0:0:0:1` are one address,
        // and so are an IPv4 address and its IPv4-mapped IPv6 form).
        let seen = ip.to_canonical();
        if self
            .ips
            .iter()
            .filter_map(|i| i.parse::<std::net::IpAddr>().ok())
            .any(|i| i.to_canonical() == seen)
        {
            Ok(())
        } else {
            Err(BindingError::Ip)
        }
    }
}

/// The claims of an entitlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claims {
    /// Schema version ([`SCHEMA_VERSION`]).
    pub v: u32,
    /// Who issued it (the authority's name).
    pub iss: String,
    /// Who it is for (`builtpanel`).
    pub aud: String,
    /// The license.
    pub license_id: Uuid,
    /// The installation it was issued to.
    pub installation_id: Uuid,
    /// What it switches on.
    pub features: Vec<String>,
    /// Numeric limits (`max_nodes`, `max_tenants`); `-1` is no limit.
    pub limits: BTreeMap<String, i64>,
    /// Where the license may be used: the domains, host names and addresses it is bound to.
    pub bind: Binding,
    /// Issued at (Unix seconds).
    pub iat: i64,
    /// Valid until (Unix seconds).
    pub exp: i64,
    /// This document's id.
    pub jti: Uuid,
    /// The key that signed it.
    pub kid: String,
}

/// Why a document was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    /// The claims could not be made canonical.
    #[error("claims: {0}")]
    Canonical(#[from] CanonicalError),
    /// The key could not sign.
    #[error("signing failed")]
    Sign,
    /// Not a document: unreadable, or not in the canonical form.
    #[error("not a valid document")]
    Malformed,
    /// Signed by a key that is not trusted, or the signature does not match.
    #[error("the signature is not valid")]
    Signature,
    /// For another issuer, audience or installation.
    #[error("the document is not for this installation")]
    Wrong,
    /// Not valid yet.
    #[error("the document is not valid yet")]
    NotYet,
    /// Out of date.
    #[error("the document has expired")]
    Expired,
    /// The license is bound to another host name, domain or address.
    #[error(transparent)]
    Binding(#[from] BindingError),
}

/// A signed entitlement, as it travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// The key id.
    pub kid: String,
    /// `EdDSA`.
    pub alg: String,
    /// The canonical claims, base64url.
    pub payload: String,
    /// The signature, base64url.
    pub signature: String,
}

fn signed_bytes(domain: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut m = domain.to_vec();
    m.extend_from_slice(payload);
    m
}

impl Document {
    /// Signs `claims` (their `kid` is set to the key's).
    ///
    /// # Errors
    /// [`DocumentError`].
    pub fn sign(claims: &Claims, key: &dyn SigningKey) -> Result<Self, DocumentError> {
        let mut claims = claims.clone();
        claims.kid = key.kid().to_owned();
        let value = serde_json::to_value(&claims).map_err(|_| DocumentError::Malformed)?;
        let payload = canonical_json(&value)?;
        let signature = key
            .sign(&signed_bytes(ENTITLEMENT_DOMAIN, &payload))
            .map_err(|_| DocumentError::Sign)?;
        Ok(Self {
            kid: claims.kid,
            alg: "EdDSA".to_owned(),
            payload: URL_SAFE_NO_PAD.encode(&payload),
            signature: URL_SAFE_NO_PAD.encode(signature),
        })
    }

    /// Checks the signature and the claims and returns them.
    ///
    /// # Errors
    /// [`DocumentError`].
    pub fn verify(
        &self,
        keys: &PublicKeys,
        expect: &Expect<'_>,
        now: i64,
    ) -> Result<Claims, DocumentError> {
        if self.alg != "EdDSA" {
            return Err(DocumentError::Malformed);
        }
        let payload = URL_SAFE_NO_PAD
            .decode(&self.payload)
            .map_err(|_| DocumentError::Malformed)?;
        let signature = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| DocumentError::Malformed)?;
        if !keys.verify(
            &self.kid,
            &signed_bytes(ENTITLEMENT_DOMAIN, &payload),
            &signature,
        ) {
            return Err(DocumentError::Signature);
        }
        let claims: Claims =
            serde_json::from_slice(&payload).map_err(|_| DocumentError::Malformed)?;
        // What was signed must be the canonical form (the one way to write it), so a copy that
        // was re-serialised or padded is not accepted, and the key id inside is the one outside.
        let value = serde_json::to_value(&claims).map_err(|_| DocumentError::Malformed)?;
        if canonical_json(&value)? != payload
            || claims.kid != self.kid
            || claims.v != SCHEMA_VERSION
        {
            return Err(DocumentError::Malformed);
        }
        if claims.iss != expect.issuer
            || claims.aud != expect.audience
            || claims.installation_id != expect.installation_id
        {
            return Err(DocumentError::Wrong);
        }
        claims.bind.check_host(&expect.host)?;
        if claims.iat > now + expect.clock_skew {
            return Err(DocumentError::NotYet);
        }
        if claims.exp <= now {
            return Err(DocumentError::Expired);
        }
        Ok(claims)
    }
}

/// What a verifier expects the claims to say.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    /// The issuer's name.
    pub issuer: &'a str,
    /// The audience.
    pub audience: &'a str,
    /// This installation.
    pub installation_id: Uuid,
    /// How far ahead of this clock `iat` may be (seconds).
    pub clock_skew: i64,
    /// What this host says about itself; the license must be bound to it.
    pub host: HostFacts<'a>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::keys::FileSigningKey;

    fn claims(installation: Uuid) -> Claims {
        Claims {
            v: SCHEMA_VERSION,
            iss: "builtpanel-la".into(),
            aud: "builtpanel".into(),
            license_id: Uuid::from_u128(1),
            installation_id: installation,
            features: vec!["resellers".into(), "mail".into()],
            limits: BTreeMap::from([("max_nodes".into(), 3), ("max_tenants".into(), -1)]),
            bind: Binding {
                domains: vec!["panel.example.com".into()],
                hostnames: vec!["web1".into()],
                ips: vec!["203.0.113.7".into(), "2001:db8::7".into()],
            },
            iat: 1_000,
            exp: 2_000,
            jti: Uuid::from_u128(2),
            kid: String::new(),
        }
    }

    fn setup() -> (FileSigningKey, PublicKeys, Uuid) {
        let dir = std::env::temp_dir().join(format!("bp-license-doc-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = FileSigningKey::generate_to_file(&dir.join("k"), "lic-1").unwrap();
        let keys = PublicKeys::new().with("lic-1", key.public_key()).unwrap();
        (key, keys, Uuid::from_u128(77))
    }

    fn expect(installation: Uuid) -> Expect<'static> {
        Expect {
            issuer: "builtpanel-la",
            audience: "builtpanel",
            installation_id: installation,
            clock_skew: 60,
            host: HostFacts {
                hostname: "web1",
                domain: "panel.example.com",
            },
        }
    }

    #[test]
    fn a_license_works_only_where_it_is_bound() {
        let (key, keys, inst) = setup();
        let doc = Document::sign(&claims(inst), &key).unwrap();
        let on = |hostname, domain| Expect {
            host: HostFacts { hostname, domain },
            ..expect(inst)
        };
        // Case and a trailing dot do not matter; a sub-domain of a bound domain does fit.
        doc.verify(&keys, &on("WEB1", "Panel.Example.com."), 1_500)
            .unwrap();
        doc.verify(&keys, &on("web1", "a.panel.example.com"), 1_500)
            .unwrap();
        assert_eq!(
            doc.verify(&keys, &on("web2", "panel.example.com"), 1_500),
            Err(DocumentError::Binding(BindingError::Hostname))
        );
        for domain in ["other.example.com", "xpanel.example.com", "example.com"] {
            assert_eq!(
                doc.verify(&keys, &on("web1", domain), 1_500),
                Err(DocumentError::Binding(BindingError::Domain)),
                "{domain}"
            );
        }
    }

    #[test]
    fn a_license_that_is_bound_to_nothing_is_not_used_and_addresses_compare_as_addresses() {
        let (key, keys, inst) = setup();
        for empty in ["domains", "hostnames", "ips"] {
            let mut c = claims(inst);
            match empty {
                "domains" => c.bind.domains.clear(),
                "hostnames" => c.bind.hostnames.clear(),
                _ => c.bind.ips.clear(),
            }
            let doc = Document::sign(&c, &key).unwrap();
            assert_eq!(
                doc.verify(&keys, &expect(inst), 1_500),
                Err(DocumentError::Binding(BindingError::Unbound)),
                "{empty}"
            );
        }
        let bind = claims(inst).bind;
        let ip = |t: &str| t.parse::<std::net::IpAddr>().unwrap();
        bind.check_ip(&ip("203.0.113.7")).unwrap();
        bind.check_ip(&ip("::ffff:203.0.113.7")).unwrap();
        bind.check_ip(&ip("2001:0db8:0:0:0:0:0:7")).unwrap();
        assert_eq!(bind.check_ip(&ip("203.0.113.8")), Err(BindingError::Ip));
        assert_eq!(bind.check_ip(&ip("127.0.0.1")), Err(BindingError::Ip));
    }

    #[test]
    fn a_signed_document_verifies_and_gives_the_claims_back() {
        let (key, keys, inst) = setup();
        let doc = Document::sign(&claims(inst), &key).unwrap();
        let got = doc.verify(&keys, &expect(inst), 1_500).unwrap();
        assert_eq!(got.features, ["resellers", "mail"]);
        assert_eq!(got.kid, "lic-1");
        // The same claims always make the same document.
        assert_eq!(doc, Document::sign(&claims(inst), &key).unwrap());
    }

    #[test]
    fn a_forged_or_changed_document_is_refused() {
        let (key, keys, inst) = setup();
        let doc = Document::sign(&claims(inst), &key).unwrap();
        // A changed claim (more nodes), with the old signature.
        let mut forged = claims(inst);
        forged.limits.insert("max_nodes".into(), 300);
        forged.kid = "lic-1".into();
        let payload = canonical_json(&serde_json::to_value(&forged).unwrap()).unwrap();
        let changed = Document {
            payload: URL_SAFE_NO_PAD.encode(&payload),
            ..doc.clone()
        };
        assert_eq!(
            changed.verify(&keys, &expect(inst), 1_500),
            Err(DocumentError::Signature)
        );
        // A flipped bit anywhere in the payload or the signature.
        for field in 0..2 {
            let mut d = doc.clone();
            let target = if field == 0 {
                &mut d.payload
            } else {
                &mut d.signature
            };
            let mut bytes = target.clone().into_bytes();
            bytes[4] = if bytes[4] == b'A' { b'B' } else { b'A' };
            *target = String::from_utf8(bytes).unwrap();
            assert!(d.verify(&keys, &expect(inst), 1_500).is_err());
        }
        // Signed by a key that is not in the bundle, or a key id that was swapped.
        let (other_key, _, _) = setup();
        let stranger = Document::sign(&claims(inst), &other_key).unwrap();
        assert_eq!(
            stranger.verify(&keys, &expect(inst), 1_500),
            Err(DocumentError::Signature)
        );
        let swapped = Document {
            kid: "lic-2".into(),
            ..doc.clone()
        };
        assert_eq!(
            swapped.verify(&keys, &expect(inst), 1_500),
            Err(DocumentError::Signature)
        );
        // A correct signature over claims that are not in the canonical form is not accepted.
        let loose = serde_json::to_vec_pretty(
            &serde_json::to_value({
                let mut c = claims(inst);
                c.kid = "lic-1".into();
                c
            })
            .unwrap(),
        )
        .unwrap();
        let sig = key.sign(&signed_bytes(ENTITLEMENT_DOMAIN, &loose)).unwrap();
        let padded = Document {
            kid: "lic-1".into(),
            alg: "EdDSA".into(),
            payload: URL_SAFE_NO_PAD.encode(&loose),
            signature: URL_SAFE_NO_PAD.encode(sig),
        };
        assert_eq!(
            padded.verify(&keys, &expect(inst), 1_500),
            Err(DocumentError::Malformed)
        );
        // A signature made for another purpose does not pass as an entitlement.
        let other_purpose = key
            .sign(&signed_bytes(b"builtpanel-release-v1\n", &payload))
            .unwrap();
        let reused = Document {
            signature: URL_SAFE_NO_PAD.encode(other_purpose),
            ..changed
        };
        assert_eq!(
            reused.verify(&keys, &expect(inst), 1_500),
            Err(DocumentError::Signature)
        );
    }

    #[test]
    fn a_document_for_another_installation_or_time_is_refused() {
        let (key, keys, inst) = setup();
        let doc = Document::sign(&claims(inst), &key).unwrap();
        assert_eq!(
            doc.verify(&keys, &expect(Uuid::from_u128(78)), 1_500),
            Err(DocumentError::Wrong)
        );
        let mut wrong_audience = expect(inst);
        wrong_audience.audience = "something-else";
        assert_eq!(
            doc.verify(&keys, &wrong_audience, 1_500),
            Err(DocumentError::Wrong)
        );
        assert_eq!(
            doc.verify(&keys, &expect(inst), 2_000),
            Err(DocumentError::Expired)
        );
        assert_eq!(
            doc.verify(&keys, &expect(inst), 100),
            Err(DocumentError::NotYet)
        );
        assert!(
            doc.verify(&keys, &expect(inst), 950).is_ok(),
            "a little clock skew is fine"
        );
    }
}
