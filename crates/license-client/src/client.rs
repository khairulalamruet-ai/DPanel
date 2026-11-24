//! Talking to the license authority (T17.09). Blocking (run it on a blocking thread).
//!
//! Every answer that carries an entitlement is checked against the trusted public keys before
//! it is handed back: what the authority's server says is not trusted on its own, the
//! signature is.

use std::time::Duration;

use builtpanel_license::{Claims, Document, DocumentError, Expect, PublicKeys};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{Identity, IdentityError};

/// The authority's answer to an activation or a refresh, checked.
#[derive(Debug, Clone)]
pub struct Received {
    /// The signed document, to keep.
    pub document: Document,
    /// Its claims, verified.
    pub claims: Claims,
    /// The license in short.
    pub license: LicenseSummary,
    /// The authority's clock (Unix seconds).
    pub server_time: i64,
}

/// A license in short, as the authority describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseSummary {
    /// Id.
    pub id: Uuid,
    /// The plan's name.
    pub plan: String,
    /// `active`, `suspended` or `revoked`.
    pub status: String,
    /// When it ends (Unix seconds).
    pub expires_at: Option<i64>,
}

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The authority could not be reached, or had a server error: try again later.
    #[error("the license authority cannot be reached: {0}")]
    Unreachable(String),
    /// The authority said no, with a code (`license_revoked`, `activation_limit`, ...).
    #[error("the license authority refused: {code}")]
    Refused {
        /// The HTTP status.
        status: u16,
        /// The problem code.
        code: String,
    },
    /// The authority's answer is not what it should be (not JSON, or not for this
    /// installation, or not signed by a trusted key).
    #[error("the answer is not valid: {0}")]
    Invalid(String),
    /// The installation's own identity is unusable.
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

impl ClientError {
    /// Whether this is the authority saying the license is no longer good (as opposed to
    /// trouble reaching it).
    #[must_use]
    pub fn is_license_refusal(&self) -> bool {
        matches!(self, Self::Refused { code, .. }
            if matches!(code.as_str(), "license_revoked" | "license_suspended" | "license_expired" | "binding_mismatch" | "integrity_failed"))
    }
}

/// The answer to the authority's domain check (see [`LicenseClient::domain_proof`]).
#[derive(Debug, Clone, Serialize)]
pub struct DomainProof {
    /// This installation.
    pub installation_id: Uuid,
    /// The domain signed for.
    pub domain: String,
    /// The signature, base64url.
    pub signature: String,
}

/// A client of one authority, for one installation.
pub struct LicenseClient {
    base: String,
    keys: PublicKeys,
    identity: Identity,
    issuer: String,
    audience: String,
    /// The machine's host name and the domain the panel is reached at: sent with every
    /// request the license is bound to, and checked against the entitlement that comes back.
    hostname: String,
    domain: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for LicenseClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LicenseClient")
            .field("base", &self.base)
            .field("installation", &self.identity.installation_id)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Challenge {
    nonce: String,
}

#[derive(Deserialize)]
struct Granted {
    entitlement: Document,
    license: LicenseSummary,
    server_time: i64,
}

/// A scan the authority asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScanChallenge {
    /// This scan.
    pub scan_id: Uuid,
    /// What the report is signed with.
    pub nonce: String,
    /// Until when it may be answered (Unix seconds).
    pub expires_at: i64,
    /// The files to hash.
    pub components: Vec<String>,
}

/// How a scan came out.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScanResult {
    /// `passed`, `failed`, `partial` or `unverifiable`.
    pub result: String,
    /// The files that are not what the release says.
    pub mismatched: Vec<String>,
    /// The files that were not reported.
    pub missing: Vec<String>,
    /// The installation's integrity state now: `ok`, `failed` or `recovering`.
    #[serde(default = "integrity_ok")]
    pub integrity: String,
}

/// What a heartbeat answers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Beat {
    /// `active`, `suspended`, `revoked` or `expired`.
    pub license_status: String,
    /// The installation's integrity state: `ok`, `failed` or `recovering`.
    #[serde(default = "integrity_ok")]
    pub integrity: String,
    /// The authority's clock.
    pub server_time: i64,
}

fn integrity_ok() -> String {
    "ok".to_owned()
}

#[derive(Deserialize)]
struct Problem {
    #[serde(default)]
    code: String,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl LicenseClient {
    /// A client for the authority at `base` (an `https://` address; `http://` only for
    /// loopback, which is what tests use).
    ///
    /// # Errors
    /// [`ClientError::Invalid`] for an address that is neither.
    pub fn new(
        base: &str,
        keys: PublicKeys,
        identity: Identity,
        issuer: &str,
        audience: &str,
    ) -> Result<Self, ClientError> {
        let base = base.trim_end_matches('/').to_owned();
        let loopback = base.starts_with("http://127.0.0.1")
            || base.starts_with("http://localhost")
            || base.starts_with("http://[::1]");
        if !(base.starts_with("https://") || loopback) {
            return Err(ClientError::Invalid(
                "the authority's address must be https".into(),
            ));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            base,
            keys,
            identity,
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            hostname: String::new(),
            domain: String::new(),
            agent,
        })
    }

    /// Says what this host is: its host name and the domain the panel is reached at. A license
    /// is bound to both, so a client without them is refused by the authority.
    #[must_use]
    pub fn with_host(mut self, hostname: &str, domain: &str) -> Self {
        self.hostname = hostname.to_owned();
        self.domain = domain.to_owned();
        self
    }

    /// The answer to the authority's domain check: this installation's signature of the
    /// authority's `nonce` for the domain it is served at.
    ///
    /// # Errors
    /// [`ClientError::Invalid`] when no domain is configured; [`ClientError::Identity`].
    pub fn domain_proof(&self, nonce: &str) -> Result<DomainProof, ClientError> {
        let domain = builtpanel_license::normalize_name(&self.domain);
        if domain.is_empty() {
            return Err(ClientError::Invalid("no domain is configured".into()));
        }
        let signature = self.identity.prove("domain", nonce, Some(&domain))?;
        Ok(DomainProof {
            installation_id: self.identity.installation_id,
            domain,
            signature,
        })
    }

    /// What is signed together with the nonce, so that the host name and domain a request
    /// states cannot be changed on the way.
    fn host_proof(&self) -> String {
        format!(
            "{}\n{}",
            builtpanel_license::normalize_name(&self.hostname),
            builtpanel_license::normalize_name(&self.domain)
        )
    }

    /// This installation's id.
    #[must_use]
    pub fn installation_id(&self) -> Uuid {
        self.identity.installation_id
    }

    fn post(&self, path: &str, body: &serde_json::Value) -> Result<(u16, String), ClientError> {
        let url = format!("{}{path}", self.base);
        let mut response = self
            .agent
            .post(&url)
            .send_json(body)
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        if status >= 500 {
            return Err(ClientError::Unreachable(format!(
                "the authority answered {status}"
            )));
        }
        if status >= 400 {
            let code = serde_json::from_str::<Problem>(&text)
                .map(|p| p.code)
                .unwrap_or_default();
            return Err(ClientError::Refused { status, code });
        }
        Ok((status, text))
    }

    fn challenge(&self) -> Result<String, ClientError> {
        let (_, text) = self.post(
            "/api/v1/licenses/challenge",
            &serde_json::json!({ "installation_id": self.identity.installation_id }),
        )?;
        serde_json::from_str::<Challenge>(&text)
            .map(|c| c.nonce)
            .map_err(|_| ClientError::Invalid("the challenge is unreadable".into()))
    }

    fn proof(&self, purpose: &str) -> Result<serde_json::Value, ClientError> {
        let nonce = self.challenge()?;
        let signature = self
            .identity
            .prove(purpose, &nonce, Some(&self.host_proof()))?;
        Ok(serde_json::json!({
            "installation_id": self.identity.installation_id,
            "nonce": nonce,
            "signature": signature,
            "hostname": self.hostname,
            "domain": self.domain,
        }))
    }

    fn received(&self, text: &str) -> Result<Received, ClientError> {
        let granted: Granted = serde_json::from_str(text)
            .map_err(|_| ClientError::Invalid("the answer is unreadable".into()))?;
        let expect = Expect {
            issuer: &self.issuer,
            audience: &self.audience,
            installation_id: self.identity.installation_id,
            clock_skew: 300,
            host: builtpanel_license::HostFacts {
                hostname: &self.hostname,
                domain: &self.domain,
            },
        };
        let claims = granted
            .entitlement
            .verify(&self.keys, &expect, unix_now())
            .map_err(|e: DocumentError| ClientError::Invalid(e.to_string()))?;
        Ok(Received {
            document: granted.entitlement,
            claims,
            license: granted.license,
            server_time: granted.server_time,
        })
    }

    /// Activates this installation with a license key.
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn activate(&self, license_key: &str) -> Result<Received, ClientError> {
        use sha2_hex::hex_sha256;
        let nonce = self.challenge()?;
        let key = license_key.trim();
        let signature = self.identity.prove(
            "activate",
            &nonce,
            Some(&format!("{}\n{}", hex_sha256(key), self.host_proof())),
        )?;
        let body = serde_json::json!({
            "license_key": key,
            "installation_id": self.identity.installation_id,
            "public_key": self.identity.public_key(),
            "nonce": nonce,
            "signature": signature,
            "hostname": self.hostname,
            "domain": self.domain,
        });
        let (_, text) = self.post("/api/v1/licenses/activate", &body)?;
        self.received(&text)
    }

    /// Asks for a fresh entitlement.
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn refresh(&self) -> Result<Received, ClientError> {
        let (_, text) = self.post("/api/v1/licenses/refresh", &self.proof("refresh")?)?;
        self.received(&text)
    }

    /// Tells the authority this installation is alive (with a little about itself).
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn heartbeat(&self, info: &serde_json::Value) -> Result<Beat, ClientError> {
        let mut body = self.proof("heartbeat")?;
        body["info"] = info.clone();
        let (_, text) = self.post("/api/v1/licenses/heartbeat", &body)?;
        serde_json::from_str(&text)
            .map_err(|_| ClientError::Invalid("the answer is unreadable".into()))
    }

    /// Lets go of the license (freeing its place).
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn deactivate(&self) -> Result<(), ClientError> {
        let url = format!("{}/api/v1/licenses/deactivate", self.base);
        let mut response = self
            .agent
            .post(&url)
            .send_json(self.proof("deactivate")?)
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        let status = response.status().as_u16();
        if status >= 500 {
            return Err(ClientError::Unreachable(format!(
                "the authority answered {status}"
            )));
        }
        if status >= 400 {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            let code = serde_json::from_str::<Problem>(&text)
                .map(|p| p.code)
                .unwrap_or_default();
            return Err(ClientError::Refused { status, code });
        }
        Ok(())
    }

    /// Asks the authority for an integrity scan of the release `version`.
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn integrity_challenge(&self, version: &str) -> Result<ScanChallenge, ClientError> {
        let mut body = self.proof("integrity")?;
        body["version"] = version.into();
        let (_, text) = self.post("/api/v1/integrity/challenge", &body)?;
        serde_json::from_str(&text)
            .map_err(|_| ClientError::Invalid("the challenge is unreadable".into()))
    }

    /// Sends what was found for a scan: file name to SHA-256 (hex), signed with the
    /// installation's key under the scan's nonce.
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn integrity_report(
        &self,
        scan: &ScanChallenge,
        files: &std::collections::BTreeMap<String, String>,
    ) -> Result<ScanResult, ClientError> {
        let signature = self.identity.prove(
            "integrity-report",
            &scan.nonce,
            Some(&builtpanel_license::integrity_report_text(
                &scan.scan_id.to_string(),
                files,
            )),
        )?;
        let (_, text) = self.post(
            "/api/v1/integrity/report",
            &serde_json::json!({
                "installation_id": self.identity.installation_id,
                "scan_id": scan.scan_id,
                "files": files,
                "signature": signature,
            }),
        )?;
        serde_json::from_str(&text)
            .map_err(|_| ClientError::Invalid("the answer is unreadable".into()))
    }

    /// Asks for help after a failed integrity check or a lost machine (a person reads it).
    ///
    /// # Errors
    /// [`ClientError`].
    pub fn recovery_request(&self, note: &str) -> Result<(), ClientError> {
        let mut body = self.proof("recovery")?;
        body["note"] = note.chars().take(2000).collect::<String>().into();
        self.post("/api/v1/licenses/recovery-request", &body)
            .map(|_| ())
    }
}

mod sha2_hex {
    //! The hex SHA-256 the activation proof covers (the authority computes the same).
    use std::fmt::Write as _;

    /// `text`'s SHA-256 in lower-case hex.
    pub fn hex_sha256(text: &str) -> String {
        let digest = builtpanel_license::sha256(text.as_bytes());
        digest.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}
