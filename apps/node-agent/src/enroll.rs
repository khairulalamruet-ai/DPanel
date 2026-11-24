//! Enrolment: trade a one-time token for a certificate.
//!
//! What happens, in this order:
//! 1. Check the arguments and that this node can store an identity, **before**
//!    the token is used, so a mistake here does not burn it.
//! 2. Make a fresh ECDSA P-256 key and a certificate signing request. The key
//!    stays on this machine.
//! 3. One HTTPS POST to the control plane. No redirects (the token must not be
//!    forwarded anywhere), a timeout, a response size limit.
//! 4. Check the answer with [`crate::verify`] against the pinned CA fingerprint
//!    and our own key.
//! 5. Only then store key, certificate, CA and identity ([`crate::identity`]).
//!
//! The token and the private key never appear in output or error messages.

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_node_protocol::{ENROLL_PATH, EnrollRequest, EnrollResponse};
use rcgen::{CertificateParams, DnType, KeyPair};
use secrecy::{ExposeSecret as _, SecretString};
use time::OffsetDateTime;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::{
    AGENT_VERSION,
    identity::{Identity, Material, StateDir, StateError},
    verify::{VerifyError, normalise_fingerprint, verify_enrollment},
};

/// Longest answer the agent will read, in bytes.
pub const MAX_RESPONSE_BYTES: u64 = 1 << 20;
/// Time allowed for the whole request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything enrolment needs.
pub struct Options {
    /// Control plane address, e.g. `https://panel.example.com:8443`.
    pub server: String,
    /// The one-time token.
    pub token: SecretString,
    /// Expected CA fingerprint (`sha256:<hex>` or plain hex).
    pub ca_fingerprint: String,
    /// Where to store the identity.
    pub state_dir: PathBuf,
    /// Host name to report; detected when absent.
    pub hostname: Option<String>,
    /// Replace an existing identity.
    pub replace: bool,
    /// Extra root certificate (PEM) to trust for this one request, for private
    /// CAs and local testing. Without it the bundled public roots are used.
    pub tls_ca_pem: Option<Vec<u8>>,
}

/// Why enrolment failed.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    /// `--server` is not acceptable.
    #[error("--server: {0}")]
    InvalidServer(String),
    /// `--tls-ca-file` could not be used.
    #[error("--tls-ca-file: {0}")]
    TlsCa(String),
    /// Key or request generation failed.
    #[error("cannot create the key or certificate request: {0}")]
    Crypto(String),
    /// Could not reach the control plane (DNS, connection, TLS, timeout).
    #[error("cannot reach the control plane: {0}")]
    Network(String),
    /// The control plane said no.
    #[error("the control plane refused ({status}{code}): {detail}", code = fmt_code(code))]
    Rejected {
        /// HTTP status.
        status: u16,
        /// Stable error code, when given.
        code: Option<String>,
        /// Explanation from the control plane.
        detail: String,
        /// Seconds to wait, when given.
        retry_after: Option<u64>,
    },
    /// The answer could not be read.
    #[error("unexpected answer from the control plane: {0}")]
    BadResponse(String),
    /// The answer failed verification; nothing was stored.
    #[error(transparent)]
    Verify(#[from] VerifyError),
    /// The identity could not be stored.
    #[error(transparent)]
    State(#[from] StateError),
}

fn fmt_code(code: &Option<String>) -> String {
    code.as_ref().map(|c| format!(" {c}")).unwrap_or_default()
}

/// Checks `--server`: `https://host[:port][/path]`, or `http://` for loopback
/// only (so a token never crosses a network in the clear). Returns it without a
/// trailing slash.
///
/// # Errors
/// [`EnrollError::InvalidServer`].
pub fn normalise_server(server: &str) -> Result<String, EnrollError> {
    let url = server.trim().trim_end_matches('/');
    let bad = |why: &str| Err(EnrollError::InvalidServer(why.to_owned()));
    // An allow-list: letters, digits and `.-_~/:` are all a host, port and path need.
    // This rules out spaces, backslashes, quotes, user info, queries and fragments.
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '~' | '/' | ':');
    if url.len() > 200 || !url.chars().all(allowed) {
        return bad(
            "use only letters, digits and . - _ ~ / : (no user info, query or fragment), at most 200 characters",
        );
    }
    if let Some(host) = url.strip_prefix("https://") {
        return if host.is_empty() || host.starts_with(['/', ':']) {
            bad("the host name is missing")
        } else {
            Ok(url.to_owned())
        };
    }
    let loopback = ["http://localhost", "http://127.0.0.1"]
        .iter()
        .any(|prefix| {
            url.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
    if loopback {
        Ok(url.to_owned())
    } else {
        bad("use https:// (plain http is only accepted for localhost)")
    }
}

fn detect_hostname() -> Option<String> {
    let raw = std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    let name = raw.trim();
    let plausible = (1..=253).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    plausible.then(|| name.to_owned())
}

fn build_agent(url: &str, tls_ca_pem: Option<&[u8]>) -> Result<ureq::Agent, EnrollError> {
    let mut tls = TlsConfig::builder();
    if let Some(pem) = tls_ca_pem {
        let certificate = Certificate::from_pem(pem)
            .map_err(|_| EnrollError::TlsCa("not a PEM certificate".to_owned()))?;
        tls = tls.root_certs(RootCerts::Specific(Arc::new(vec![certificate])));
    }
    Ok(ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(REQUEST_TIMEOUT))
        // A redirect would re-send the token to wherever it points.
        .max_redirects(0)
        .https_only(url.starts_with("https://"))
        .user_agent(format!("builtpanel-agent/{AGENT_VERSION}"))
        .tls_config(tls.build())
        .build()
        .into())
}

/// Turns a refusal into an error, reading the Problem Details body if there is one.
fn rejection(status: u16, body: &str, retry_after: Option<u64>) -> EnrollError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let field = |name: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let mut detail = field("detail").unwrap_or_else(|| "no explanation given".to_owned());
    // 422 carries per-field problems; show them, they are what the user must fix.
    if let Some(errors) = parsed
        .as_ref()
        .and_then(|v| v.get("errors"))
        .and_then(|e| e.as_array())
    {
        let list: Vec<String> = errors
            .iter()
            .filter_map(|e| {
                Some(format!(
                    "{}: {}",
                    e.get("field")?.as_str()?,
                    e.get("message")?.as_str()?
                ))
            })
            .collect();
        if !list.is_empty() {
            detail = format!("{detail} ({})", list.join("; "));
        }
    }
    EnrollError::Rejected {
        status,
        code: field("code"),
        detail,
        retry_after,
    }
}

/// What the stored identity looks like after a successful enrolment.
#[derive(Debug, Clone)]
pub struct Enrolled {
    /// The stored identity.
    pub identity: Identity,
}

/// Enrols this node. See the module documentation for the steps.
///
/// # Errors
/// [`EnrollError`]; on any error before the final storing step, nothing is
/// written, and errors up to and including the request leave the token unused
/// unless the control plane itself consumed it.
pub fn enroll(options: &Options) -> Result<Enrolled, EnrollError> {
    // 1. Everything that can be checked without the network.
    let server = normalise_server(&options.server)?;
    let pin = normalise_fingerprint(&options.ca_fingerprint)?;
    let state = StateDir::new(&options.state_dir);
    state.prepare()?;
    if state.is_enrolled() && !options.replace {
        return Err(StateError::AlreadyEnrolled(options.state_dir.clone()).into());
    }
    state.ensure_writable()?;
    let agent = build_agent(&server, options.tls_ca_pem.as_deref())?;

    // 2. A fresh key and a request for it. The names in the request are ignored
    //    by the control plane; it decides the identity.
    let key = KeyPair::generate().map_err(|e| EnrollError::Crypto(e.to_string()))?;
    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| EnrollError::Crypto(e.to_string()))?;
    params
        .distinguished_name
        .push(DnType::CommonName, "builtpanel-agent-enrolling");
    let csr_pem = params
        .serialize_request(&key)
        .and_then(|csr| csr.pem())
        .map_err(|e| EnrollError::Crypto(e.to_string()))?;

    // 3. The one network call.
    let request = EnrollRequest {
        token: options.token.expose_secret().to_owned(),
        csr_pem,
        hostname: options.hostname.clone().or_else(detect_hostname),
        agent_version: Some(AGENT_VERSION.to_owned()),
    };
    let url = format!("{server}{ENROLL_PATH}");
    let mut response = agent
        .post(&url)
        .header("accept", "application/json")
        .send_json(&request)
        .map_err(|e| EnrollError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|e| EnrollError::Network(e.to_string()))?;
    if status != 201 {
        return Err(rejection(status, &body, retry_after));
    }
    let answer: EnrollResponse = serde_json::from_str(&body)
        .map_err(|_| EnrollError::BadResponse("the answer is not the expected JSON".to_owned()))?;

    // 4. Trust nothing in it until it is checked.
    let now = OffsetDateTime::now_utc();
    let verified = verify_enrollment(&answer, key.public_key_raw(), &pin, now.unix_timestamp())?;

    // 5. Store.
    let identity = Identity {
        node_id: answer.node_id,
        name: answer.name.clone(),
        server,
        ca_fingerprint_sha256: pin,
        enrolled_at: now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        certificate_not_after: verified.not_after,
        protocol_version: answer.protocol_version,
        stream_url: answer
            .stream_url
            .clone()
            .filter(|url| crate::stream::valid_stream_url(url)),
    };
    state.save(
        &Material {
            key_pem: &SecretString::from(key.serialize_pem()),
            cert_pem: &answer.certificate_pem,
            ca_pem: &answer.ca_certificate_pem,
            identity: &identity,
        },
        options.replace,
    )?;
    Ok(Enrolled { identity })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn servers_must_be_https_or_loopback() {
        for (input, expected) in [
            ("https://panel.example.com", "https://panel.example.com"),
            (
                " https://panel.example.com:8443/ ",
                "https://panel.example.com:8443",
            ),
            ("https://10.0.0.5:8443/agent", "https://10.0.0.5:8443/agent"),
            ("http://localhost:8080", "http://localhost:8080"),
            ("http://127.0.0.1", "http://127.0.0.1"),
        ] {
            assert_eq!(normalise_server(input).unwrap(), expected, "{input:?}");
        }
        for bad in [
            "",
            "panel.example.com",
            "http://panel.example.com",
            "http://localhost.evil.com",
            "ftp://x.example.com",
            "https://",
            "https:///x",
            "https://:8443",
            "https://user@host.example",
            "https://host.example/?x=1",
            "https://host.example/#f",
            "https://host .example",
            "https://host.example\\path",
            "https://ho\nst.example",
            "https://host.example/a\"b",
            "https://[::1]:8443",
            &format!("https://{}", "a".repeat(200)),
        ] {
            assert!(
                matches!(normalise_server(bad), Err(EnrollError::InvalidServer(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn refusals_show_the_code_and_the_field_problems() {
        let body = r#"{"type":"x","title":"Unauthorized","status":401,"detail":"The enrollment token is not valid.","code":"invalid_enrollment_token"}"#;
        let err = rejection(401, body, None);
        assert_eq!(
            err.to_string(),
            "the control plane refused (401 invalid_enrollment_token): The enrollment token is not valid."
        );
        let body = r#"{"detail":"One or more fields are invalid.","code":"validation_failed","errors":[{"field":"hostname","code":"invalid_format","message":"Use letters."}]}"#;
        assert!(
            rejection(422, body, None)
                .to_string()
                .contains("hostname: Use letters.")
        );
        // Not JSON at all (a proxy's error page): still a clear error.
        let err = rejection(502, "<html>Bad gateway</html>", Some(7));
        assert!(
            matches!(
                &err,
                EnrollError::Rejected {
                    status: 502,
                    code: None,
                    retry_after: Some(7),
                    ..
                }
            ),
            "{err}"
        );
        assert!(err.to_string().contains("no explanation given"));
    }

    #[test]
    fn a_pem_that_is_not_a_certificate_is_a_clear_error() {
        assert!(matches!(
            build_agent("https://x.example", Some(b"nope")),
            Err(EnrollError::TlsCa(_))
        ));
        assert!(build_agent("https://x.example", None).is_ok());
    }
}
