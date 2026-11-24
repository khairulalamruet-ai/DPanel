//! TLS for the agent port: TLS 1.3 only, client certificate required.

use std::{fs, path::Path, sync::Arc};

use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _},
    server::WebPkiClientVerifier,
    version::TLS13,
};

/// File names of the control plane's server certificate inside `pki.ca_dir`
/// (written by `bpctl ca issue-server`).
pub const SERVER_CERT_FILE: &str = "server.crt";
/// See [`SERVER_CERT_FILE`].
pub const SERVER_KEY_FILE: &str = "server.key";

/// Why the TLS configuration could not be built.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file.
        path: String,
        /// Why.
        source: std::io::Error,
    },
    /// The private key file can be read by other users.
    #[error("{path} is readable by other users (mode {mode:o}); run: chmod 600 {path}")]
    InsecureKey {
        /// The file.
        path: String,
        /// Its permission bits.
        mode: u32,
    },
    /// The PEM text or the certificates in it are unusable.
    #[error("{0}")]
    Invalid(String),
}

fn invalid(what: &str, err: impl std::fmt::Display) -> TlsError {
    TlsError::Invalid(format!("{what}: {err}"))
}

/// Builds the server configuration. `ca_pem` is the node CA certificate: it is
/// the only issuer clients are trusted from. `cert_pem` is the server's chain
/// and `key_pem` its private key.
///
/// # Errors
/// [`TlsError::Invalid`] when a PEM block is missing, malformed, or the key does
/// not match the certificate.
pub fn server_config(
    ca_pem: &str,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<Arc<ServerConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        let cert = cert.map_err(|e| invalid("CA certificate", e))?;
        roots.add(cert).map_err(|e| invalid("CA certificate", e))?;
    }
    if roots.is_empty() {
        return Err(TlsError::Invalid("no CA certificate found".into()));
    }
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| invalid("client verifier", e))?;
    let chain = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid("server certificate", e))?;
    if chain.is_empty() {
        return Err(TlsError::Invalid("no server certificate found".into()));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| invalid("server key", e))?;
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&TLS13])
        .map_err(|e| invalid("TLS versions", e))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|e| invalid("server certificate and key", e))?;
    Ok(Arc::new(config))
}

/// Loads `server.crt` and `server.key` from `dir` and builds the configuration.
/// The key file must not be readable by group or others.
///
/// # Errors
/// [`TlsError`].
pub fn load_server_config(ca_pem: &str, dir: &Path) -> Result<Arc<ServerConfig>, TlsError> {
    let (cert_path, key_path) = (dir.join(SERVER_CERT_FILE), dir.join(SERVER_KEY_FILE));
    let read = |path: &Path| {
        fs::read(path).map_err(|source| TlsError::Io {
            path: path.display().to_string(),
            source,
        })
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&key_path)
            .map_err(|source| TlsError::Io {
                path: key_path.display().to_string(),
                source,
            })?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(TlsError::InsecureKey {
                path: key_path.display().to_string(),
                mode,
            });
        }
    }
    server_config(ca_pem, &read(&cert_path)?, &read(&key_path)?)
}
