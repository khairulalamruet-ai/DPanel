//! Internal certificate authority for node mTLS (ADR 0004).
//!
//! One CA signs two kinds of certificates:
//! * **node certificates** for agents (client authentication), issued from a
//!   certificate signing request (CSR) the agent made with its own key. The
//!   CSR only proves the agent holds that key: **everything else in it is
//!   ignored**. The name (`node-<uuid>`), the identity URI
//!   (`urn:builtpanel:node:<uuid>`), the key usages, "not a CA" and the validity
//!   are set here, so an agent cannot ask for another node's identity, a server
//!   role or CA powers.
//! * **server certificates** for the control plane's agent-facing port.
//!
//! Keys are ECDSA P-256. Serial numbers are random 127-bit values. Certificates
//! are backdated by 5 minutes to tolerate clock skew. The CA private key is only
//! ever written to a file created with mode 0600 and is never logged.

pub mod custom;

use std::{
    fs,
    io::{self, Write as _},
    net::IpAddr,
    path::{Path, PathBuf},
};

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, SanType, SerialNumber, string::Ia5String,
};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// File name of the CA certificate inside the CA directory.
pub const CA_CERT_FILE: &str = "node_ca.crt";
/// File name of the CA private key inside the CA directory.
pub const CA_KEY_FILE: &str = "node_ca.key";
/// Prefix of the identity URI in node certificates: `urn:builtpanel:node:<uuid>`.
pub const NODE_URI_PREFIX: &str = "urn:builtpanel:node:";
/// Default lifetime of node certificates (ADR 0004: 90 days, rotated by the agent).
pub const DEFAULT_NODE_VALIDITY: Duration = Duration::days(90);
/// Longest lifetime allowed for a node or server certificate.
pub const MAX_LEAF_VALIDITY: Duration = Duration::days(400);
/// Longest lifetime allowed for the CA certificate.
pub const MAX_CA_VALIDITY: Duration = Duration::days(20 * 365);
/// Shortest lifetime allowed for any certificate.
pub const MIN_VALIDITY: Duration = Duration::hours(1);
/// How far `not_before` is moved into the past.
pub const BACKDATE: Duration = Duration::minutes(5);

/// Errors from this crate. Messages never contain key material.
#[derive(Debug, thiserror::Error)]
pub enum PkiError {
    /// A certificate library operation failed.
    #[error("certificate error: {0}")]
    Certificate(String),
    /// The CSR could not be parsed or its signature does not verify.
    #[error("invalid certificate signing request: {0}")]
    InvalidCsr(String),
    /// The CSR's key is not ECDSA P-256.
    #[error("unsupported key type: agents must use ECDSA P-256")]
    UnsupportedKey,
    /// The CA files do not describe a usable CA.
    #[error("invalid CA: {0}")]
    InvalidCa(String),
    /// A requested value is out of range or malformed.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// CA files already exist; they are never overwritten.
    #[error("a CA already exists in {0}; refusing to overwrite it")]
    AlreadyInitialised(PathBuf),
    /// The CA key file is readable by others.
    #[error("{path}: permissions {mode:o} are too open; use 0600 or 0400")]
    InsecureKeyPermissions {
        /// The offending file.
        path: PathBuf,
        /// Its mode bits.
        mode: u32,
    },
    /// OS randomness is unavailable.
    #[error("random number generator unavailable")]
    Rng,
    /// File system error.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
}

fn cert_err(e: impl std::fmt::Display) -> PkiError {
    PkiError::Certificate(e.to_string())
}

/// A certificate the CA issued, with the facts the control plane stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCertificate {
    /// The certificate, PEM encoded.
    pub cert_pem: String,
    /// Serial number as 32 lowercase hex digits.
    pub serial_hex: String,
    /// SHA-256 of the DER encoding, lowercase hex. This is how a node is
    /// recognised during the mTLS handshake.
    pub fingerprint_sha256: String,
    /// Start of validity (Unix seconds).
    pub not_before: i64,
    /// End of validity (Unix seconds).
    pub not_after: i64,
    /// Subject common name.
    pub subject_cn: String,
}

/// The certificate authority: its certificate and signing key.
pub struct CertificateAuthority {
    issuer: Issuer<'static, KeyPair>,
    cert_pem: String,
    fingerprint: String,
    subject_cn: String,
    not_before: i64,
    not_after: i64,
}

impl std::fmt::Debug for CertificateAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateAuthority")
            .field("subject_cn", &self.subject_cn)
            .field("fingerprint_sha256", &self.fingerprint)
            .field("not_after", &self.not_after)
            .finish_non_exhaustive()
    }
}

fn check_validity(valid_for: Duration, max: Duration) -> Result<(), PkiError> {
    if valid_for < MIN_VALIDITY || valid_for > max {
        return Err(PkiError::InvalidInput(format!(
            "validity must be between {} hour and {} days",
            MIN_VALIDITY.whole_hours(),
            max.whole_days()
        )));
    }
    Ok(())
}

/// 16 random bytes whose top byte is 0x40..=0x7f: always positive, always
/// 16 bytes long in DER.
fn random_serial() -> Result<[u8; 16], PkiError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| PkiError::Rng)?;
    bytes[0] = (bytes[0] & 0x7f) | 0x40;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn fingerprint(der: &[u8]) -> String {
    hex(&Sha256::digest(der))
}

fn dn(common_name: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "Built Panel");
    dn.push(DnType::CommonName, common_name);
    dn
}

/// Formats Unix seconds as `2026-11-12T08:30:00Z`.
#[must_use]
pub fn format_rfc3339(unix_seconds: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

impl CertificateAuthority {
    /// Creates a new CA with a fresh P-256 key. Returns the CA and its private
    /// key in PEM form; the caller must store the key safely (see
    /// [`init_ca_dir`]).
    ///
    /// # Errors
    /// Invalid name or validity, RNG or certificate library failures.
    pub fn create(
        common_name: &str,
        now: OffsetDateTime,
        valid_for: Duration,
    ) -> Result<(Self, SecretString), PkiError> {
        let name = common_name.trim();
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err(PkiError::InvalidInput(
                "CA name must be 1-64 printable characters".into(),
            ));
        }
        check_validity(valid_for, MAX_CA_VALIDITY)?;
        let key = KeyPair::generate().map_err(cert_err)?;
        let mut params = CertificateParams::default();
        params.distinguished_name = dn(name);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.not_before = now - BACKDATE;
        params.not_after = now + valid_for;
        params.serial_number = Some(SerialNumber::from_slice(&random_serial()?));
        params.use_authority_key_identifier_extension = true;
        let cert = params.self_signed(&key).map_err(cert_err)?;
        let key_pem = SecretString::from(key.serialize_pem());
        let ca = Self::from_parts(cert.pem(), &key_pem, now)?;
        Ok((ca, key_pem))
    }

    /// Loads a CA and checks that it really is one: self-signed, `CA:TRUE`,
    /// allowed to sign certificates, not expired, and matching the private key.
    ///
    /// # Errors
    /// [`PkiError::InvalidCa`] for any of the above, or parse failures.
    pub fn from_pem(
        cert_pem: &str,
        key_pem: &SecretString,
        now: OffsetDateTime,
    ) -> Result<Self, PkiError> {
        Self::from_parts(cert_pem.to_owned(), key_pem, now)
    }

    fn from_parts(
        cert_pem: String,
        key_pem: &SecretString,
        now: OffsetDateTime,
    ) -> Result<Self, PkiError> {
        use x509_parser::prelude::{FromDer as _, X509Certificate};

        let key = KeyPair::from_pem(key_pem.expose_secret())
            .map_err(|_| PkiError::InvalidCa("the CA key cannot be read".into()))?;
        let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
            .map_err(|_| PkiError::InvalidCa("the CA certificate is not valid PEM".into()))?;
        let (_, cert) = X509Certificate::from_der(&pem.contents)
            .map_err(|_| PkiError::InvalidCa("the CA certificate cannot be parsed".into()))?;
        let bad = |why: &str| PkiError::InvalidCa(why.into());

        if !cert.is_ca() {
            return Err(bad("the certificate is not a CA (CA:TRUE is missing)"));
        }
        if !cert
            .key_usage()
            .ok()
            .flatten()
            .is_some_and(|k| k.value.key_cert_sign())
        {
            return Err(bad("the certificate may not sign certificates"));
        }
        cert.verify_signature(None)
            .map_err(|_| bad("the certificate is not correctly self-signed"))?;
        if cert.public_key().subject_public_key.data.as_ref() != key.public_key_raw() {
            return Err(bad("the private key does not belong to this certificate"));
        }
        let validity = cert.validity();
        if now.unix_timestamp() < validity.not_before.timestamp() {
            return Err(bad("the CA certificate is not valid yet"));
        }
        if now.unix_timestamp() >= validity.not_after.timestamp() {
            return Err(bad("the CA certificate has expired"));
        }
        let subject_cn = cert
            .subject()
            .iter_common_name()
            .next()
            .and_then(|cn| cn.as_str().ok())
            .unwrap_or_default()
            .to_owned();
        let (not_before, not_after) = (
            validity.not_before.timestamp(),
            validity.not_after.timestamp(),
        );
        let fingerprint = fingerprint(&pem.contents);

        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key).map_err(cert_err)?;
        Ok(Self {
            issuer,
            cert_pem,
            fingerprint,
            subject_cn,
            not_before,
            not_after,
        })
    }

    /// The CA certificate, PEM encoded (public; give it to agents).
    #[must_use]
    pub fn certificate_pem(&self) -> &str {
        &self.cert_pem
    }

    /// SHA-256 fingerprint of the CA certificate, lowercase hex.
    #[must_use]
    pub fn fingerprint_sha256(&self) -> &str {
        &self.fingerprint
    }

    /// The CA's common name.
    #[must_use]
    pub fn common_name(&self) -> &str {
        &self.subject_cn
    }

    /// Start of the CA certificate's validity (Unix seconds).
    #[must_use]
    pub fn not_before(&self) -> i64 {
        self.not_before
    }

    /// End of the CA certificate's validity (Unix seconds).
    #[must_use]
    pub fn not_after(&self) -> i64 {
        self.not_after
    }

    fn leaf_params(
        common_name: &str,
        sans: Vec<SanType>,
        usage: ExtendedKeyUsagePurpose,
        now: OffsetDateTime,
        valid_for: Duration,
    ) -> Result<CertificateParams, PkiError> {
        check_validity(valid_for, MAX_LEAF_VALIDITY)?;
        let mut params = CertificateParams::default();
        params.distinguished_name = dn(common_name);
        params.subject_alt_names = sans;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![usage];
        params.not_before = now - BACKDATE;
        params.not_after = now + valid_for;
        params.serial_number = Some(SerialNumber::from_slice(&random_serial()?));
        params.use_authority_key_identifier_extension = true;
        Ok(params)
    }

    fn describe(
        &self,
        params: &CertificateParams,
        cert: &rcgen::Certificate,
        common_name: String,
    ) -> IssuedCertificate {
        IssuedCertificate {
            cert_pem: cert.pem(),
            serial_hex: params
                .serial_number
                .as_ref()
                .map(|s| hex(s.as_ref()))
                .unwrap_or_default(),
            fingerprint_sha256: fingerprint(cert.der()),
            not_before: params.not_before.unix_timestamp(),
            not_after: params.not_after.unix_timestamp(),
            subject_cn: common_name,
        }
    }

    /// Issues a client certificate for a node from its CSR.
    ///
    /// Only the CSR's public key is used, after its self-signature has been
    /// verified (proof that the agent holds the key). Names, key usages and CA
    /// flags the CSR asks for are discarded.
    ///
    /// # Errors
    /// [`PkiError::InvalidCsr`], [`PkiError::UnsupportedKey`] for anything but
    /// ECDSA P-256, [`PkiError::InvalidInput`] for a bad validity.
    pub fn issue_node_certificate(
        &self,
        csr_pem: &str,
        node_id: Uuid,
        now: OffsetDateTime,
        valid_for: Duration,
    ) -> Result<IssuedCertificate, PkiError> {
        let csr = CertificateSigningRequestParams::from_pem(csr_pem)
            .map_err(|e| PkiError::InvalidCsr(e.to_string()))?;
        if csr.public_key.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            return Err(PkiError::UnsupportedKey);
        }
        let cn = format!("node-{node_id}");
        let uri = Ia5String::try_from(format!("{NODE_URI_PREFIX}{node_id}")).map_err(cert_err)?;
        let params = Self::leaf_params(
            &cn,
            vec![SanType::URI(uri)],
            ExtendedKeyUsagePurpose::ClientAuth,
            now,
            valid_for,
        )?;
        let cert = params
            .signed_by(&csr.public_key, &self.issuer)
            .map_err(cert_err)?;
        Ok(self.describe(&params, &cert, cn))
    }

    /// Issues a server certificate (with a freshly generated key) for the
    /// control plane's agent-facing port. `names` are DNS names or IP
    /// addresses; the first one becomes the common name. Wildcards are refused.
    ///
    /// # Errors
    /// [`PkiError::InvalidInput`] for bad names or validity.
    pub fn issue_server_certificate(
        &self,
        names: &[String],
        now: OffsetDateTime,
        valid_for: Duration,
    ) -> Result<(IssuedCertificate, SecretString), PkiError> {
        if names.is_empty() || names.len() > 20 {
            return Err(PkiError::InvalidInput(
                "give between 1 and 20 DNS names or IP addresses".into(),
            ));
        }
        let mut sans = Vec::with_capacity(names.len());
        for name in names {
            sans.push(parse_server_name(name)?);
        }
        let cn = names[0].trim().to_ascii_lowercase();
        let params = Self::leaf_params(
            &cn,
            sans,
            ExtendedKeyUsagePurpose::ServerAuth,
            now,
            valid_for,
        )?;
        let key = KeyPair::generate().map_err(cert_err)?;
        let cert = params.signed_by(&key, &self.issuer).map_err(cert_err)?;
        Ok((
            self.describe(&params, &cert, cn),
            SecretString::from(key.serialize_pem()),
        ))
    }
}

/// Accepts IP addresses and plain host names (letters, digits, hyphens and
/// dots; 253 characters, labels of at most 63 and not starting or ending with
/// a hyphen). Wildcards and underscores are refused.
fn parse_server_name(raw: &str) -> Result<SanType, PkiError> {
    let name = raw.trim().to_ascii_lowercase();
    if let Ok(ip) = name.parse::<IpAddr>() {
        return Ok(SanType::IpAddress(ip));
    }
    let bad = || PkiError::InvalidInput(format!("`{raw}` is not a valid host name or IP address"));
    let well_formed = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if !well_formed {
        return Err(bad());
    }
    Ok(SanType::DnsName(
        Ia5String::try_from(name).map_err(|_| bad())?,
    ))
}

// ------------------------------------------------------------------- files

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> PkiError + '_ {
    move |source| PkiError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Creates a new file, failing if it exists, readable only by the owner.
///
/// # Errors
/// [`PkiError::Io`] (including "already exists").
pub fn write_private_file(path: &Path, contents: &str) -> Result<(), PkiError> {
    write_new(path, contents, 0o600)
}

/// Creates a new file readable by everyone (certificates), failing if it exists.
///
/// # Errors
/// [`PkiError::Io`] (including "already exists").
pub fn write_public_file(path: &Path, contents: &str) -> Result<(), PkiError> {
    write_new(path, contents, 0o644)
}

fn write_new(path: &Path, contents: &str, mode: u32) -> Result<(), PkiError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path).map_err(io_err(path))?;
    file.write_all(contents.as_bytes()).map_err(io_err(path))?;
    file.sync_all().map_err(io_err(path))
}

/// Creates the CA in `dir` (made with mode 0700 if missing): `node_ca.key`
/// (0600) and `node_ca.crt` (0644). Never overwrites an existing CA.
///
/// # Errors
/// [`PkiError::AlreadyInitialised`] if either file exists, plus everything
/// [`CertificateAuthority::create`] can return and file system errors.
pub fn init_ca_dir(
    dir: &Path,
    common_name: &str,
    now: OffsetDateTime,
    valid_for: Duration,
) -> Result<CertificateAuthority, PkiError> {
    let (cert_path, key_path) = (dir.join(CA_CERT_FILE), dir.join(CA_KEY_FILE));
    if cert_path.exists() || key_path.exists() {
        return Err(PkiError::AlreadyInitialised(dir.to_owned()));
    }
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io_err(dir))?;
        }
    }
    let (ca, key_pem) = CertificateAuthority::create(common_name, now, valid_for)?;
    // Key first: if writing the certificate fails there is no half-visible CA.
    write_private_file(&key_path, key_pem.expose_secret())?;
    if let Err(e) = write_public_file(&cert_path, ca.certificate_pem()) {
        let _ = fs::remove_file(&key_path);
        return Err(e);
    }
    Ok(ca)
}

/// Loads the CA from `dir`, insisting that the key file is not readable by
/// group or others.
///
/// # Errors
/// [`PkiError::InsecureKeyPermissions`], file system errors and everything
/// [`CertificateAuthority::from_pem`] can return.
pub fn load_ca_dir(dir: &Path, now: OffsetDateTime) -> Result<CertificateAuthority, PkiError> {
    let (cert_path, key_path) = (dir.join(CA_CERT_FILE), dir.join(CA_KEY_FILE));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&key_path)
            .map_err(io_err(&key_path))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(PkiError::InsecureKeyPermissions {
                path: key_path,
                mode,
            });
        }
    }
    let cert_pem = fs::read_to_string(&cert_path).map_err(io_err(&cert_path))?;
    let key_pem = SecretString::from(fs::read_to_string(&key_path).map_err(io_err(&key_path))?);
    CertificateAuthority::from_pem(&cert_pem, &key_pem, now)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use rcgen::CertificateParams as P;
    use x509_parser::prelude::{FromDer as _, GeneralName, ParsedExtension, X509Certificate};

    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
    }

    fn new_ca() -> (CertificateAuthority, SecretString) {
        CertificateAuthority::create("Test CA", now(), Duration::days(3650)).unwrap()
    }

    fn der(pem: &str) -> Vec<u8> {
        x509_parser::pem::parse_x509_pem(pem.as_bytes())
            .unwrap()
            .1
            .contents
    }

    /// An agent's key and the CSR it would send. The CSR is deliberately greedy:
    /// it asks for CA powers, a server role and somebody else's name.
    fn greedy_csr() -> (KeyPair, String) {
        let key = KeyPair::generate().unwrap();
        let mut params = P::new(vec!["evil.example.com".to_owned()]).unwrap();
        params.distinguished_name.push(DnType::CommonName, "root");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let csr = params.serialize_request(&key).unwrap().pem().unwrap();
        (key, csr)
    }

    fn parse(der: &[u8]) -> X509Certificate<'_> {
        X509Certificate::from_der(der).unwrap().1
    }

    #[test]
    fn the_ca_is_a_constrained_self_signed_ca() {
        let (ca, _) = new_ca();
        let der = der(ca.certificate_pem());
        let cert = parse(&der);
        assert!(cert.is_ca());
        let bc = cert.basic_constraints().unwrap().unwrap().value;
        assert!(
            bc.ca && bc.path_len_constraint == Some(0),
            "no intermediate CAs below it"
        );
        let ku = cert.key_usage().unwrap().unwrap().value;
        assert!(ku.key_cert_sign() && ku.crl_sign() && !ku.digital_signature());
        cert.verify_signature(None).expect("self-signed");
        assert_eq!(cert.subject(), cert.issuer());
        assert_eq!(ca.common_name(), "Test CA");
        assert_eq!(cert.serial.to_bytes_be().len(), 16);
        assert_eq!(ca.not_after() - ca.not_before(), 3650 * 86_400 + 300);
        assert_eq!(ca.fingerprint_sha256().len(), 64);
        assert!(format!("{ca:?}").contains("Test CA") && !format!("{ca:?}").contains("PRIVATE"));
    }

    #[test]
    fn node_certificates_get_only_what_the_ca_decides() {
        let (ca, _) = new_ca();
        let (agent_key, csr) = greedy_csr();
        let node = Uuid::now_v7();
        let issued = ca
            .issue_node_certificate(&csr, node, now(), DEFAULT_NODE_VALIDITY)
            .unwrap();
        let der = der(&issued.cert_pem);
        let cert = parse(&der);

        // Identity and role are ours, not the CSR's.
        assert_eq!(issued.subject_cn, format!("node-{node}"));
        assert_eq!(
            cert.subject()
                .iter_common_name()
                .next()
                .unwrap()
                .as_str()
                .unwrap(),
            format!("node-{node}")
        );
        let sans: Vec<String> = cert
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value
            .general_names
            .iter()
            .map(|n| match n {
                GeneralName::URI(u) => format!("uri:{u}"),
                GeneralName::DNSName(d) => format!("dns:{d}"),
                other => format!("other:{other:?}"),
            })
            .collect();
        assert_eq!(
            sans,
            [format!("uri:{NODE_URI_PREFIX}{node}")],
            "requested DNS names are dropped"
        );
        assert!(!cert.is_ca(), "no CA powers");
        assert!(!cert.basic_constraints().unwrap().unwrap().value.ca);
        let ku = cert.key_usage().unwrap().unwrap().value;
        assert!(ku.digital_signature() && !ku.key_cert_sign());
        let eku = cert.extended_key_usage().unwrap().unwrap().value;
        assert!(eku.client_auth && !eku.server_auth, "client role only");

        // Validity: 90 days plus the 5 minute backdating.
        assert_eq!(issued.not_after - issued.not_before, 90 * 86_400 + 300);
        assert_eq!(issued.not_before, now().unix_timestamp() - 300);
        assert_eq!(cert.validity().not_after.timestamp(), issued.not_after);

        // Signed by the CA, bound to the agent's key, with an authority key id.
        let ca_der = der_of_ca(&ca);
        cert.verify_signature(Some(parse(&ca_der).public_key()))
            .expect("signed by the CA");
        assert!(cert.verify_signature(None).is_err(), "not self-signed");
        assert_eq!(
            cert.public_key().subject_public_key.data.as_ref(),
            agent_key.public_key_raw()
        );
        assert!(cert.extensions().iter().any(|e| matches!(
            e.parsed_extension(),
            ParsedExtension::AuthorityKeyIdentifier(_)
        )));

        // Recorded facts match the certificate.
        assert_eq!(issued.fingerprint_sha256, fingerprint(&der));
        assert_eq!(issued.serial_hex.len(), 32);
        assert_eq!(issued.serial_hex, hex(&cert.serial.to_bytes_be()));
    }

    fn der_of_ca(ca: &CertificateAuthority) -> Vec<u8> {
        der(ca.certificate_pem())
    }

    #[test]
    fn serials_are_unique_positive_and_sixteen_bytes() {
        let (ca, _) = new_ca();
        let (_, csr) = greedy_csr();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let c = ca
                .issue_node_certificate(&csr, Uuid::now_v7(), now(), DEFAULT_NODE_VALIDITY)
                .unwrap();
            assert!(seen.insert(c.serial_hex.clone()), "duplicate serial");
            assert!(
                u8::from_str_radix(&c.serial_hex[..2], 16).unwrap() & 0x80 == 0,
                "positive"
            );
            assert_eq!(c.serial_hex.len(), 32);
        }
    }

    #[test]
    fn bad_csrs_and_keys_are_refused() {
        let (ca, _) = new_ca();
        let issue = |csr: &str| {
            ca.issue_node_certificate(csr, Uuid::now_v7(), now(), DEFAULT_NODE_VALIDITY)
        };
        for garbage in [
            "",
            "not pem",
            "-----BEGIN CERTIFICATE REQUEST-----\nAAAA\n-----END CERTIFICATE REQUEST-----\n",
        ] {
            assert!(
                matches!(issue(garbage), Err(PkiError::InvalidCsr(_))),
                "{garbage:?}"
            );
        }
        // A valid CSR whose signature has been damaged: no proof of key possession.
        let (_, good) = greedy_csr();
        let mut bytes = good.clone().into_bytes();
        let last = bytes.iter().rposition(|b| b.is_ascii_alphabetic()).unwrap();
        let line_end = bytes[..last].iter().rposition(|b| *b == b'\n').unwrap();
        let target = line_end - 3; // inside the last base64 line, i.e. in the signature
        bytes[target] = if bytes[target] == b'A' { b'B' } else { b'A' };
        assert!(
            issue(std::str::from_utf8(&bytes).unwrap()).is_err(),
            "tampered CSR"
        );
        // A certificate is not a CSR.
        assert!(issue(ca.certificate_pem()).is_err());
        // Only ECDSA P-256 keys are accepted.
        let ed = KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let csr = P::new(Vec::<String>::new())
            .unwrap()
            .serialize_request(&ed)
            .unwrap()
            .pem()
            .unwrap();
        assert!(matches!(issue(&csr), Err(PkiError::UnsupportedKey)));
    }

    #[test]
    fn validity_limits_are_enforced() {
        let (ca, _) = new_ca();
        let (_, csr) = greedy_csr();
        for d in [
            Duration::ZERO,
            Duration::minutes(59),
            MAX_LEAF_VALIDITY + Duration::seconds(1),
            Duration::days(-1),
        ] {
            assert!(
                matches!(
                    ca.issue_node_certificate(&csr, Uuid::now_v7(), now(), d),
                    Err(PkiError::InvalidInput(_))
                ),
                "{d}"
            );
            assert!(
                ca.issue_server_certificate(&["a.example.com".into()], now(), d)
                    .is_err(),
                "{d}"
            );
        }
        assert!(
            ca.issue_node_certificate(&csr, Uuid::now_v7(), now(), MAX_LEAF_VALIDITY)
                .is_ok()
        );
        assert!(
            CertificateAuthority::create("x", now(), MAX_CA_VALIDITY + Duration::days(1)).is_err()
        );
        for name in ["", "   ", &"n".repeat(65), "bad\nname"] {
            assert!(
                CertificateAuthority::create(name, now(), Duration::days(30)).is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn server_certificates_carry_the_requested_names_and_a_matching_key() {
        let (ca, _) = new_ca();
        let names = [
            "Control.Example.COM".to_owned(),
            "10.0.0.5".to_owned(),
            "::1".to_owned(),
        ];
        let (issued, key_pem) = ca
            .issue_server_certificate(&names, now(), Duration::days(365))
            .unwrap();
        assert_eq!(issued.subject_cn, "control.example.com");
        let der = der(&issued.cert_pem);
        let cert = parse(&der);
        let sans = &cert
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value
            .general_names;
        assert!(
            sans.iter()
                .any(|n| matches!(n, GeneralName::DNSName("control.example.com")))
        );
        assert!(
            sans.iter()
                .any(|n| matches!(n, GeneralName::IPAddress(b) if *b == [10, 0, 0, 5]))
        );
        assert_eq!(sans.len(), 3);
        let eku = cert.extended_key_usage().unwrap().unwrap().value;
        assert!(eku.server_auth && !eku.client_auth);
        assert!(!cert.is_ca());
        cert.verify_signature(Some(parse(&der_of_ca(&ca)).public_key()))
            .unwrap();
        let key = KeyPair::from_pem(key_pem.expose_secret()).unwrap();
        assert_eq!(
            cert.public_key().subject_public_key.data.as_ref(),
            key.public_key_raw()
        );
        assert!(!format!("{issued:?}").contains("PRIVATE"));
    }

    #[test]
    fn unsafe_server_names_are_refused() {
        let (ca, _) = new_ca();
        for bad in [
            "",
            "*.example.com",
            "under_score.example.com",
            "-lead.example.com",
            "trail-.example.com",
            "a..b",
            "exa mple.com",
            "bücher.example",
            &format!("{}.com", "a".repeat(64)),
            &"a.".repeat(130),
        ] {
            assert!(
                matches!(
                    ca.issue_server_certificate(&[bad.to_owned()], now(), Duration::days(30)),
                    Err(PkiError::InvalidInput(_))
                ),
                "{bad:?}"
            );
        }
        assert!(
            ca.issue_server_certificate(&[], now(), Duration::days(30))
                .is_err()
        );
        assert!(
            ca.issue_server_certificate(
                &vec!["a.example.com".to_owned(); 21],
                now(),
                Duration::days(30)
            )
            .is_err()
        );
    }

    #[test]
    fn a_ca_reloads_only_if_it_is_a_real_matching_unexpired_ca() {
        let (ca, key) = new_ca();
        let again = CertificateAuthority::from_pem(ca.certificate_pem(), &key, now()).unwrap();
        assert_eq!(again.fingerprint_sha256(), ca.fingerprint_sha256());
        // A reloaded CA signs certificates the original's certificate verifies.
        let (_, csr) = greedy_csr();
        let c = again
            .issue_node_certificate(&csr, Uuid::now_v7(), now(), DEFAULT_NODE_VALIDITY)
            .unwrap();
        parse(&der(&c.cert_pem))
            .verify_signature(Some(parse(&der_of_ca(&ca)).public_key()))
            .unwrap();

        // Someone else's key.
        let (_, other_key) = new_ca();
        let err =
            CertificateAuthority::from_pem(ca.certificate_pem(), &other_key, now()).unwrap_err();
        assert!(
            matches!(&err, PkiError::InvalidCa(m) if m.contains("does not belong")),
            "{err}"
        );
        // A leaf certificate is not a CA, even with its own matching key.
        let (leaf, leaf_key) = ca
            .issue_server_certificate(&["a.example.com".into()], now(), Duration::days(30))
            .unwrap();
        let err = CertificateAuthority::from_pem(&leaf.cert_pem, &leaf_key, now()).unwrap_err();
        assert!(
            matches!(&err, PkiError::InvalidCa(m) if m.contains("not a CA")),
            "{err}"
        );
        // Expired and not yet valid.
        let late = now() + Duration::days(3651);
        assert!(
            matches!(&CertificateAuthority::from_pem(ca.certificate_pem(), &key, late).unwrap_err(),
            PkiError::InvalidCa(m) if m.contains("expired"))
        );
        let early = now() - Duration::days(1);
        assert!(CertificateAuthority::from_pem(ca.certificate_pem(), &key, early).is_err());
        // Garbage.
        assert!(CertificateAuthority::from_pem("nope", &key, now()).is_err());
        assert!(
            CertificateAuthority::from_pem(
                ca.certificate_pem(),
                &SecretString::from("nope".to_owned()),
                now()
            )
            .is_err()
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("builtpanel-pki-{tag}-{}", Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn the_ca_directory_is_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = temp_dir("init");
        let dir = base.join("secrets");
        let ca = init_ca_dir(&dir, "Disk CA", now(), Duration::days(3650)).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "new directory is private");
        assert_eq!(mode(&dir.join(CA_KEY_FILE)), 0o600);
        assert_eq!(mode(&dir.join(CA_CERT_FILE)), 0o644);
        let key_text = fs::read_to_string(dir.join(CA_KEY_FILE)).unwrap();
        assert!(key_text.contains("PRIVATE KEY"));
        assert!(
            !fs::read_to_string(dir.join(CA_CERT_FILE))
                .unwrap()
                .contains("PRIVATE")
        );

        // Second init: refused, and the files are untouched.
        let before = fs::read(dir.join(CA_KEY_FILE)).unwrap();
        assert!(matches!(
            init_ca_dir(&dir, "Other", now(), Duration::days(30)),
            Err(PkiError::AlreadyInitialised(_))
        ));
        assert_eq!(fs::read(dir.join(CA_KEY_FILE)).unwrap(), before);
        // Even a lone leftover file blocks it.
        fs::remove_file(dir.join(CA_CERT_FILE)).unwrap();
        assert!(matches!(
            init_ca_dir(&dir, "Other", now(), Duration::days(30)),
            Err(PkiError::AlreadyInitialised(_))
        ));
        fs::write(dir.join(CA_CERT_FILE), ca.certificate_pem()).unwrap();

        // Loading works, and refuses a key readable by others.
        assert_eq!(
            load_ca_dir(&dir, now()).unwrap().fingerprint_sha256(),
            ca.fingerprint_sha256()
        );
        fs::set_permissions(dir.join(CA_KEY_FILE), fs::Permissions::from_mode(0o640)).unwrap();
        let err = load_ca_dir(&dir, now()).unwrap_err();
        assert!(
            matches!(err, PkiError::InsecureKeyPermissions { mode: 0o640, .. }),
            "{err}"
        );
        assert!(!err.to_string().contains("PRIVATE"));
        fs::set_permissions(dir.join(CA_KEY_FILE), fs::Permissions::from_mode(0o400)).unwrap();
        assert!(load_ca_dir(&dir, now()).is_ok(), "0400 is fine");
        // Missing files are reported with their path.
        let err = load_ca_dir(&base.join("nothing"), now()).unwrap_err();
        assert!(err.to_string().contains("nothing"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn new_files_never_replace_existing_ones() {
        let dir = temp_dir("write");
        let path = dir.join("f");
        write_public_file(&path, "one").unwrap();
        assert!(write_public_file(&path, "two").is_err());
        assert!(write_private_file(&path, "two").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The task's DoD: a certificate signed by the CA passes `openssl verify`.
    #[test]
    fn openssl_verifies_certificates_signed_by_the_ca() {
        use std::process::Command;

        let dir = temp_dir("openssl");
        let (ca, _) = new_ca();
        let (other_ca, _) =
            CertificateAuthority::create("Other CA", now(), Duration::days(3650)).unwrap();
        let (_, csr) = greedy_csr();
        let node = ca
            .issue_node_certificate(&csr, Uuid::now_v7(), now(), DEFAULT_NODE_VALIDITY)
            .unwrap();
        let (server, _) = ca
            .issue_server_certificate(&["control.example.com".into()], now(), Duration::days(365))
            .unwrap();
        for (name, pem) in [
            ("ca.crt", ca.certificate_pem()),
            ("other.crt", other_ca.certificate_pem()),
            ("node.crt", &node.cert_pem),
            ("server.crt", &server.cert_pem),
        ] {
            fs::write(dir.join(name), pem).unwrap();
        }
        // `-attime` pins "now" to the test clock so the result does not depend on today's date.
        let verify = |ca_file: &str, cert: &str, purpose: &str| {
            let out = Command::new("openssl")
                .current_dir(&dir)
                .args([
                    "verify",
                    "-attime",
                    "1800000000",
                    "-purpose",
                    purpose,
                    "-CAfile",
                    ca_file,
                    cert,
                ])
                .output()
                .expect("the `openssl` command is required to run this test");
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).into_owned()
                    + &String::from_utf8_lossy(&out.stderr),
            )
        };
        let (ok, text) = verify("ca.crt", "node.crt", "sslclient");
        assert!(ok && text.contains("OK"), "node certificate: {text}");
        let (ok, text) = verify("ca.crt", "server.crt", "sslserver");
        assert!(ok && text.contains("OK"), "server certificate: {text}");
        // The roles are not interchangeable, and another CA's trust does not apply.
        assert!(
            !verify("ca.crt", "node.crt", "sslserver").0,
            "a node certificate is not a server certificate"
        );
        assert!(
            !verify("ca.crt", "server.crt", "sslclient").0,
            "a server certificate is not a client certificate"
        );
        assert!(
            !verify("other.crt", "node.crt", "sslclient").0,
            "a different CA must not verify it"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
