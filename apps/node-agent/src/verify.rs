//! Checking what the control plane sent back, trusting none of it.
//!
//! Enrolment answers with a certificate and a CA certificate. Before anything is
//! stored, the agent checks that:
//!
//! * the CA is the one it was told to expect (the fingerprint from the enrol
//!   command, which the administrator copied from the panel),
//! * that CA really is a self-signed CA,
//! * the node certificate was signed by it, belongs to **this agent's own key**,
//!   names **this** node, is for client authentication only, is not a CA, and is
//!   valid now for a sensible period.
//!
//! A control plane that is wrong, compromised or impersonated therefore cannot
//! hand the agent a certificate for a key it does not hold, a different CA than
//! the pinned one, or an identity with extra powers.

use builtpanel_node_protocol::EnrollResponse;
use sha2::{Digest as _, Sha256};
use x509_parser::prelude::{FromDer as _, GeneralName, X509Certificate};

/// How far in the future a certificate's start may be (clock differences).
pub const MAX_CLOCK_SKEW_SECS: i64 = 300;
/// A certificate with less life left than this is useless (the agent renews
/// long before) and is refused.
pub const MIN_REMAINING_SECS: i64 = 24 * 3600;
/// The longest lifetime a node certificate may have (400 days plus slack).
pub const MAX_LIFETIME_SECS: i64 = 401 * 24 * 3600;

/// What was checked and found good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    /// Certificate validity start (Unix seconds).
    pub not_before: i64,
    /// Certificate expiry (Unix seconds).
    pub not_after: i64,
}

/// Why a response was refused. Messages are for the administrator running the
/// enrol command and contain no secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// The expected CA fingerprint is not 64 hex digits.
    #[error(
        "the CA fingerprint must be 64 hexadecimal digits (optionally prefixed with `sha256:`)"
    )]
    BadPin,
    /// The control plane presented a different CA than the one pinned.
    #[error("the control plane's CA does not match the pinned fingerprint; refusing to trust it")]
    CaMismatch,
    /// The response contradicts itself or is unreadable.
    #[error("the control plane's answer is not usable: {0}")]
    Malformed(&'static str),
    /// The CA certificate is not a valid, current, self-signed CA.
    #[error("the CA certificate is not acceptable: {0}")]
    BadCa(&'static str),
    /// The node certificate is not acceptable.
    #[error("the node certificate is not acceptable: {0}")]
    BadCertificate(&'static str),
}

/// Normalises a fingerprint given as `sha256:<hex>` or plain hex.
///
/// # Errors
/// [`VerifyError::BadPin`].
pub fn normalise_fingerprint(input: &str) -> Result<String, VerifyError> {
    let hex = input.trim().strip_prefix("sha256:").unwrap_or(input.trim());
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(hex.to_ascii_lowercase())
    } else {
        Err(VerifyError::BadPin)
    }
}

/// SHA-256 of `der`, lowercase hex.
#[must_use]
pub fn fingerprint(der: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(der)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .ok()
        .map(|(_, p)| p.contents)
}

/// Checks the enrolment response against the pinned CA fingerprint and the
/// agent's own public key (`own_public_key`, the raw key bytes).
///
/// # Errors
/// [`VerifyError`] describing the first failed check.
pub fn verify_enrollment(
    response: &EnrollResponse,
    own_public_key: &[u8],
    pinned_ca_fingerprint: &str,
    now: i64,
) -> Result<Verified, VerifyError> {
    let pin = normalise_fingerprint(pinned_ca_fingerprint)?;

    // 1. The CA is the pinned one.
    let ca_der = pem_to_der(&response.ca_certificate_pem).ok_or(VerifyError::Malformed(
        "the CA certificate is not valid PEM",
    ))?;
    let actual = fingerprint(&ca_der);
    if actual != pin {
        return Err(VerifyError::CaMismatch);
    }
    if normalise_fingerprint(&response.ca_fingerprint_sha256).as_deref() != Ok(actual.as_str()) {
        return Err(VerifyError::Malformed(
            "the stated CA fingerprint does not match the CA certificate",
        ));
    }

    // 2. The CA is a current, self-signed CA.
    let (_, ca) = X509Certificate::from_der(&ca_der)
        .map_err(|_| VerifyError::BadCa("it cannot be parsed"))?;
    if !ca.is_ca() {
        return Err(VerifyError::BadCa("it is not a CA"));
    }
    if !ca
        .key_usage()
        .ok()
        .flatten()
        .is_some_and(|k| k.value.key_cert_sign())
    {
        return Err(VerifyError::BadCa("it may not sign certificates"));
    }
    ca.verify_signature(None)
        .map_err(|_| VerifyError::BadCa("it is not correctly self-signed"))?;
    if now >= ca.validity().not_after.timestamp()
        || now < ca.validity().not_before.timestamp() - MAX_CLOCK_SKEW_SECS
    {
        return Err(VerifyError::BadCa("it is not valid at this time"));
    }

    // 3. The node certificate.
    let cert_der = pem_to_der(&response.certificate_pem).ok_or(VerifyError::Malformed(
        "the node certificate is not valid PEM",
    ))?;
    let (_, cert) = X509Certificate::from_der(&cert_der)
        .map_err(|_| VerifyError::BadCertificate("it cannot be parsed"))?;
    cert.verify_signature(Some(ca.public_key()))
        .map_err(|_| VerifyError::BadCertificate("it was not signed by the pinned CA"))?;
    if cert.public_key().subject_public_key.data.as_ref() != own_public_key {
        return Err(VerifyError::BadCertificate(
            "it is for a different key than this agent generated",
        ));
    }
    if cert.is_ca() {
        return Err(VerifyError::BadCertificate("it has CA powers"));
    }
    let eku = cert
        .extended_key_usage()
        .ok()
        .flatten()
        .map(|e| e.value.clone())
        .ok_or(VerifyError::BadCertificate("it has no extended key usage"))?;
    if !eku.client_auth || eku.server_auth || eku.any || eku.code_signing || eku.email_protection {
        return Err(VerifyError::BadCertificate(
            "it is not for client authentication only",
        ));
    }
    let expected_uris = [format!("urn:builtpanel:node:{}", response.node_id)];
    let names_this_node = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .is_some_and(|san| {
            san.value.general_names.len() == 1
                && matches!(&san.value.general_names[0], GeneralName::URI(u) if expected_uris.iter().any(|e| e == u))
        });
    if !names_this_node {
        return Err(VerifyError::BadCertificate(
            "it does not name this node (urn:builtpanel:node:<id>)",
        ));
    }
    let (not_before, not_after) = (
        cert.validity().not_before.timestamp(),
        cert.validity().not_after.timestamp(),
    );
    if not_before > now + MAX_CLOCK_SKEW_SECS {
        return Err(VerifyError::BadCertificate(
            "it starts in the future; check this server's clock",
        ));
    }
    if not_after < now + MIN_REMAINING_SECS {
        return Err(VerifyError::BadCertificate("it expires too soon"));
    }
    if not_after - not_before > MAX_LIFETIME_SECS {
        return Err(VerifyError::BadCertificate(
            "its lifetime is unreasonably long",
        ));
    }
    Ok(Verified {
        not_before,
        not_after,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use builtpanel_pki::{CertificateAuthority, DEFAULT_NODE_VALIDITY};
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
        KeyPair, KeyUsagePurpose, SanType, string::Ia5String,
    };
    use time::{Duration, OffsetDateTime};
    use uuid::Uuid;

    use super::*;

    /// A test CA built directly with rcgen, so any flaw can be put into the
    /// certificates it issues.
    struct Fixture {
        issuer: Issuer<'static, KeyPair>,
        ca_pem: String,
        ca_fp: String,
        agent: KeyPair,
        node_id: Uuid,
        now: OffsetDateTime,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_ca(|_| {})
        }

        fn with_ca(tweak: impl FnOnce(&mut CertificateParams)) -> Self {
            let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
            let mut params = CertificateParams::default();
            params
                .distinguished_name
                .push(DnType::CommonName, "Fixture CA");
            params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            params.not_before = now - Duration::days(1);
            params.not_after = now + Duration::days(3650);
            tweak(&mut params);
            let key = KeyPair::generate().unwrap();
            let cert = params.self_signed(&key).unwrap();
            Self {
                ca_pem: cert.pem(),
                ca_fp: fingerprint(cert.der()),
                issuer: Issuer::new(params, key),
                agent: KeyPair::generate().unwrap(),
                node_id: Uuid::now_v7(),
                now,
            }
        }

        /// A leaf exactly as the real control plane issues it.
        fn good_leaf(&self) -> CertificateParams {
            let mut p = CertificateParams::default();
            p.distinguished_name
                .push(DnType::CommonName, format!("node-{}", self.node_id));
            p.subject_alt_names = vec![SanType::URI(
                Ia5String::try_from(format!("urn:builtpanel:node:{}", self.node_id)).unwrap(),
            )];
            p.is_ca = IsCa::ExplicitNoCa;
            p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
            p.not_before = self.now - Duration::minutes(5);
            p.not_after = self.now + Duration::days(90);
            p
        }

        fn respond(&self, leaf: &CertificateParams) -> EnrollResponse {
            let cert = leaf.signed_by(&self.agent, &self.issuer).unwrap();
            EnrollResponse {
                node_id: self.node_id,
                name: "n1".into(),
                certificate_pem: cert.pem(),
                ca_certificate_pem: self.ca_pem.clone(),
                ca_fingerprint_sha256: self.ca_fp.clone(),
                not_before: String::new(),
                not_after: String::new(),
                protocol_version: 1,
                stream_url: None,
            }
        }

        fn check(&self, response: &EnrollResponse) -> Result<Verified, VerifyError> {
            verify_enrollment(
                response,
                self.agent.public_key_raw(),
                &self.ca_fp,
                self.now.unix_timestamp(),
            )
        }

        /// Issues a leaf with one flaw and returns what verification says.
        fn flawed(
            &self,
            tweak: impl FnOnce(&mut CertificateParams),
        ) -> Result<Verified, VerifyError> {
            let mut leaf = self.good_leaf();
            tweak(&mut leaf);
            self.check(&self.respond(&leaf))
        }
    }

    fn reason(err: VerifyError) -> String {
        err.to_string()
    }

    #[test]
    fn a_correct_answer_is_accepted_and_the_pin_may_be_written_loosely() {
        let f = Fixture::new();
        let r = f.respond(&f.good_leaf());
        let v = f.check(&r).unwrap();
        assert_eq!(v.not_after - v.not_before, 90 * 86_400 + 300);
        for pin in [
            format!("sha256:{}", f.ca_fp),
            f.ca_fp.to_uppercase(),
            format!("  sha256:{}  ", f.ca_fp),
        ] {
            assert!(
                verify_enrollment(&r, f.agent.public_key_raw(), &pin, f.now.unix_timestamp())
                    .is_ok(),
                "{pin}"
            );
        }
    }

    #[test]
    fn what_the_real_control_plane_issues_is_accepted() {
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let (ca, _) = CertificateAuthority::create("Real CA", now, Duration::days(3650)).unwrap();
        let agent = KeyPair::generate().unwrap();
        let csr = CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .serialize_request(&agent)
            .unwrap()
            .pem()
            .unwrap();
        let node_id = Uuid::now_v7();
        let issued = ca
            .issue_node_certificate(&csr, node_id, now, DEFAULT_NODE_VALIDITY)
            .unwrap();
        let response = EnrollResponse {
            node_id,
            name: "n1".into(),
            certificate_pem: issued.cert_pem,
            ca_certificate_pem: ca.certificate_pem().to_owned(),
            ca_fingerprint_sha256: ca.fingerprint_sha256().to_owned(),
            not_before: String::new(),
            not_after: String::new(),
            protocol_version: 1,
            stream_url: None,
        };
        let v = verify_enrollment(
            &response,
            agent.public_key_raw(),
            ca.fingerprint_sha256(),
            now.unix_timestamp(),
        )
        .unwrap();
        assert_eq!(v.not_after, issued.not_after);
    }

    #[test]
    fn an_impostor_with_its_own_ca_is_refused() {
        let real = Fixture::new();
        let impostor = Fixture::new();
        // The impostor answers perfectly, but with its own CA and a certificate for the agent's key.
        let forged = EnrollResponse {
            certificate_pem: impostor
                .good_leaf()
                .signed_by(&real.agent, &impostor.issuer)
                .unwrap()
                .pem(),
            node_id: impostor.node_id,
            ..impostor.respond(&impostor.good_leaf())
        };
        let err = verify_enrollment(
            &forged,
            real.agent.public_key_raw(),
            &real.ca_fp,
            real.now.unix_timestamp(),
        )
        .unwrap_err();
        assert_eq!(err, VerifyError::CaMismatch);
    }

    #[test]
    fn a_certificate_for_another_key_or_another_signer_or_another_node_is_refused() {
        let f = Fixture::new();
        let r = f.respond(&f.good_leaf());
        // Not this agent's key.
        let other = KeyPair::generate().unwrap();
        let err = verify_enrollment(&r, other.public_key_raw(), &f.ca_fp, f.now.unix_timestamp())
            .unwrap_err();
        assert!(reason(err).contains("different key"));
        // Signed by somebody else, presented with the pinned CA's certificate.
        let rogue = Fixture::new();
        let mut forged = r.clone();
        forged.certificate_pem = f
            .good_leaf()
            .signed_by(&f.agent, &rogue.issuer)
            .unwrap()
            .pem();
        assert!(reason(f.check(&forged).unwrap_err()).contains("not signed by the pinned CA"));
        // A certificate that names a different node than the response says.
        let mut other_node = r.clone();
        other_node.node_id = Uuid::now_v7();
        assert!(reason(f.check(&other_node).unwrap_err()).contains("does not name this node"));
    }

    /// A way to damage a certificate's parameters before it is issued.
    type Flaw = Box<dyn FnOnce(&mut CertificateParams)>;

    #[test]
    fn the_certificate_must_be_a_plain_client_certificate() {
        let f = Fixture::new();
        let cases: Vec<(&str, Flaw, &str)> = vec![
            (
                "CA powers",
                Box::new(|p| p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained)),
                "CA powers",
            ),
            (
                "server role too",
                Box::new(|p| {
                    p.extended_key_usages
                        .push(ExtendedKeyUsagePurpose::ServerAuth)
                }),
                "client authentication only",
            ),
            (
                "server role only",
                Box::new(|p| p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth]),
                "client authentication only",
            ),
            (
                "any usage",
                Box::new(|p| p.extended_key_usages = vec![ExtendedKeyUsagePurpose::Any]),
                "client authentication only",
            ),
            (
                "no usage",
                Box::new(|p| p.extended_key_usages.clear()),
                "no extended key usage",
            ),
            (
                "no identity",
                Box::new(|p| p.subject_alt_names.clear()),
                "does not name this node",
            ),
            (
                "extra name",
                Box::new(|p| {
                    p.subject_alt_names.push(SanType::DnsName(
                        Ia5String::try_from("evil.example.com".to_owned()).unwrap(),
                    ))
                }),
                "does not name this node",
            ),
            (
                "another node's identity",
                Box::new(|p| {
                    p.subject_alt_names = vec![SanType::URI(
                        Ia5String::try_from(format!("urn:builtpanel:node:{}", Uuid::now_v7()))
                            .unwrap(),
                    )]
                }),
                "does not name this node",
            ),
        ];
        for (what, tweak, expected) in cases {
            let err = f.flawed(tweak).expect_err(what);
            assert!(reason(err.clone()).contains(expected), "{what}: {err}");
        }
    }

    #[test]
    fn the_validity_period_must_be_sensible() {
        let f = Fixture::new();
        let now = f.now;
        let err =
            |tweak: Box<dyn FnOnce(&mut CertificateParams)>| reason(f.flawed(tweak).unwrap_err());
        assert!(
            err(Box::new(move |p| {
                p.not_before = now - Duration::days(100);
                p.not_after = now - Duration::days(1);
            }))
            .contains("expires too soon")
        );
        assert!(
            err(Box::new(move |p| p.not_after = now + Duration::hours(12)))
                .contains("expires too soon")
        );
        assert!(
            err(Box::new(move |p| {
                p.not_before = now + Duration::days(1);
                p.not_after = now + Duration::days(90);
            }))
            .contains("starts in the future")
        );
        assert!(
            err(Box::new(move |p| p.not_after = now + Duration::days(800)))
                .contains("unreasonably long")
        );
        // Clock differences inside the allowance are fine.
        assert!(
            f.flawed(move |p| p.not_before = now + Duration::minutes(2))
                .is_ok()
        );
    }

    #[test]
    fn the_ca_itself_must_be_a_current_self_signed_ca() {
        // A leaf certificate pinned as if it were a CA.
        let f = Fixture::new();
        let leaf = f.good_leaf();
        let leaf_cert = leaf.signed_by(&f.agent, &f.issuer).unwrap();
        let mut r = f.respond(&f.good_leaf());
        r.ca_certificate_pem = leaf_cert.pem();
        r.ca_fingerprint_sha256 = fingerprint(leaf_cert.der());
        let err = verify_enrollment(
            &r,
            f.agent.public_key_raw(),
            &r.ca_fingerprint_sha256,
            f.now.unix_timestamp(),
        )
        .unwrap_err();
        assert_eq!(err, VerifyError::BadCa("it is not a CA"));
        // Expired CA, and not-yet-valid CA.
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let expired = Fixture::with_ca(move |p| {
            p.not_before = now - Duration::days(400);
            p.not_after = now - Duration::days(1);
        });
        let r = expired.respond(&expired.good_leaf());
        assert_eq!(
            expired.check(&r).unwrap_err(),
            VerifyError::BadCa("it is not valid at this time")
        );
        let future = Fixture::with_ca(move |p| {
            p.not_before = now + Duration::days(30);
            p.not_after = now + Duration::days(400);
        });
        assert_eq!(
            future
                .check(&future.respond(&future.good_leaf()))
                .unwrap_err(),
            VerifyError::BadCa("it is not valid at this time")
        );
        // A CA that may not sign certificates.
        let mute = Fixture::with_ca(|p| p.key_usages = vec![KeyUsagePurpose::CrlSign]);
        assert_eq!(
            mute.check(&mute.respond(&mute.good_leaf())).unwrap_err(),
            VerifyError::BadCa("it may not sign certificates")
        );
    }

    #[test]
    fn contradictions_and_garbage_are_refused() {
        let f = Fixture::new();
        let good = f.respond(&f.good_leaf());
        let mut lying = good.clone();
        lying.ca_fingerprint_sha256 = "00".repeat(32);
        assert!(
            matches!(f.check(&lying), Err(VerifyError::Malformed(m)) if m.contains("stated CA fingerprint"))
        );
        for (field, mutate) in [
            (
                "ca pem",
                Box::new(|r: &mut EnrollResponse| r.ca_certificate_pem = "nope".into())
                    as Box<dyn Fn(&mut EnrollResponse)>,
            ),
            ("cert pem", Box::new(|r| r.certificate_pem = "nope".into())),
            ("empty cert", Box::new(|r| r.certificate_pem.clear())),
            (
                "cert is the CA",
                Box::new(|r| r.certificate_pem = r.ca_certificate_pem.clone()),
            ),
        ] {
            let mut r = good.clone();
            mutate(&mut r);
            assert!(f.check(&r).is_err(), "{field}");
        }
        for bad_pin in ["", "xyz", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            assert_eq!(
                normalise_fingerprint(bad_pin),
                Err(VerifyError::BadPin),
                "{bad_pin:?}"
            );
            assert_eq!(
                verify_enrollment(
                    &good,
                    f.agent.public_key_raw(),
                    bad_pin,
                    f.now.unix_timestamp()
                ),
                Err(VerifyError::BadPin)
            );
        }
    }
}
