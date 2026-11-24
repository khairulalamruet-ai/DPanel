//! Fetching a backup from another server over SSH (T15.05).
//!
//! The credential (a password, or a private key and its passphrase) is held in memory only: it
//! is used for one connection and dropped (the `secrecy` types wipe it), it is never written to
//! the database, the log or an error message. The server's host key must be known: the
//! administrator gives its SHA-256 fingerprint, and a connection to a server that shows another
//! key is refused. Without a fingerprint nothing is sent to the server; the error names the
//! fingerprint it showed, so it can be checked and given.
//!
//! The panel connects to the address the name resolves to now (so the name cannot be changed
//! between the check and the connection) and refuses addresses that are not a server on the
//! network: itself, the link-local range of cloud metadata services, and unspecified or multicast
//! ones.

use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{
    client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use russh_sftp::client::SftpSession;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// How long connecting and logging in may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the connection may be silent while a file comes over.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Bytes read at a time.
const BLOCK: usize = 256 * 1024;

/// How to log in.
pub enum Credential {
    /// A password.
    Password(SecretString),
    /// A private key (OpenSSH or PEM text) and its passphrase, when it has one.
    Key {
        /// The key.
        pem: SecretString,
        /// Its passphrase.
        passphrase: Option<SecretString>,
    },
}

/// Where a backup is and how to reach it.
pub struct Source {
    /// Host name or address.
    pub host: String,
    /// SSH port.
    pub port: u16,
    /// Login name.
    pub user: String,
    /// How to log in.
    pub credential: Credential,
    /// The host key's fingerprint (`SHA256:...`) the server must show.
    pub host_key: Option<String>,
    /// The backup's path on the server.
    pub path: String,
    /// Whether addresses of this machine and its networks are allowed (tests only).
    pub allow_local: bool,
}

/// Why a fetch failed. The messages never contain the credential.
#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The address is not one this panel may connect to.
    Refused(String),
    /// The server could not be reached.
    Connect(String),
    /// No fingerprint was given: this is the one the server showed.
    UnknownHostKey(String),
    /// The server showed another key than the one given.
    HostKeyChanged {
        /// What was given.
        expected: String,
        /// What the server showed.
        got: String,
    },
    /// The login was refused.
    Login,
    /// The key could not be read.
    Key(String),
    /// The file could not be read from the server.
    Remote(String),
    /// The file is bigger than the limit.
    TooBig(u64),
    /// The file could not be written here.
    Local(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(m) => write!(f, "this address cannot be used: {m}"),
            Self::Connect(m) => write!(f, "the server could not be reached: {m}"),
            Self::UnknownHostKey(got) => write!(
                f,
                "the server's host key is {got}. If it is the right server, start the transfer again with host_key_sha256 set to it"
            ),
            Self::HostKeyChanged { expected, got } => write!(
                f,
                "the server shows the host key {got}, not {expected}: the connection was refused"
            ),
            Self::Login => write!(f, "the server refused the login"),
            Self::Key(m) => write!(f, "the private key cannot be used: {m}"),
            Self::Remote(m) => write!(f, "the backup could not be read from the server: {m}"),
            Self::TooBig(n) => write!(f, "the backup is {n} bytes, more than is allowed"),
            Self::Local(m) => write!(f, "the backup could not be saved: {m}"),
        }
    }
}

/// What a fetch brought.
#[derive(Debug, PartialEq, Eq)]
pub struct Fetched {
    /// Bytes saved.
    pub bytes: u64,
    /// The server's host key fingerprint.
    pub host_key: String,
}

/// Whether the panel may connect to an address.
#[must_use]
pub fn allowed(ip: IpAddr, allow_local: bool) -> bool {
    if allow_local {
        return !ip.is_unspecified() && !ip.is_multicast();
    }
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_link_local()
                || v4.is_broadcast())
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => allowed(IpAddr::V4(v4), false),
            None => {
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (v6.segments()[0] & 0xffc0) == 0xfe80)
            }
        },
    }
}

struct Check {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Check {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256),
            PublicKeyOrCertificate::Certificate(cert) => {
                cert.signature_key().fingerprint(HashAlg::Sha256)
            }
        }
        .to_string();
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(fingerprint.clone());
        }
        // Without the fingerprint to compare with, nothing may be sent to the server.
        Ok(self.expected.as_deref() == Some(fingerprint.as_str()))
    }
}

/// Connects, checks the host key against `host_key` and logs in. Returns the session and the key
/// fingerprint the server showed.
async fn open(
    host: &str,
    port: u16,
    user: &str,
    credential: Credential,
    host_key: Option<String>,
    allow_local: bool,
) -> Result<(client::Handle<Check>, String), FetchError> {
    let expected = host_key.map(|k| k.trim().to_owned());
    let found = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| FetchError::Connect(e.to_string()))?
        .collect::<Vec<SocketAddr>>();
    let addr = found
        .iter()
        .find(|a| allowed(a.ip(), allow_local))
        .copied()
        .ok_or_else(|| {
            FetchError::Refused(if found.is_empty() {
                "the name does not resolve".to_owned()
            } else {
                "it is this machine, a link-local address or not a server address".to_owned()
            })
        })?;
    let seen = Arc::new(Mutex::new(None));
    let config = client::Config {
        inactivity_timeout: Some(IDLE_TIMEOUT),
        ..client::Config::default()
    };
    let handler = Check {
        expected: expected.clone(),
        seen: seen.clone(),
    };
    let shown = || seen.lock().ok().and_then(|s| s.clone());
    let mut session = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect(Arc::new(config), addr, handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Ok(Err(e)) => {
            return Err(match (shown(), &expected) {
                (Some(got), None) => FetchError::UnknownHostKey(got),
                (Some(got), Some(want)) if &got != want => FetchError::HostKeyChanged {
                    expected: want.clone(),
                    got,
                },
                _ => FetchError::Connect(e.to_string()),
            });
        }
        Err(_) => return Err(FetchError::Connect("timed out".to_owned())),
    };
    let host_key = shown().unwrap_or_default();
    let login = match credential {
        Credential::Password(password) => session
            .authenticate_password(user, password.expose_secret())
            .await
            .map_err(|e| FetchError::Connect(e.to_string()))?,
        Credential::Key { pem, passphrase } => {
            let key = russh::keys::decode_secret_key(
                pem.expose_secret(),
                passphrase.as_ref().map(|p| p.expose_secret()),
            )
            .map_err(|e| FetchError::Key(e.to_string()))?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(|e| FetchError::Connect(e.to_string()))?
                .flatten();
            session
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|e| FetchError::Connect(e.to_string()))?
        }
    };
    if !login.success() {
        return Err(FetchError::Login);
    }
    Ok((session, host_key))
}

/// The host key fingerprint (`SHA256:...`) a server shows, without sending it anything: the connection is
/// refused at the key exchange, before any login.
///
/// # Errors
/// [`FetchError::Refused`] for an address the panel may not connect to, [`FetchError::Connect`] when the
/// server cannot be reached.
pub async fn probe_host_key(
    host: &str,
    port: u16,
    allow_local: bool,
) -> Result<String, FetchError> {
    match open(
        host,
        port,
        "probe",
        Credential::Password(SecretString::from(String::new())),
        None,
        allow_local,
    )
    .await
    {
        Err(FetchError::UnknownHostKey(got)) => Ok(got),
        Err(e) => Err(e),
        Ok(_) => Err(FetchError::Connect(
            "the server did not show its key".to_owned(),
        )),
    }
}

/// What a command ended with.
#[derive(Debug, PartialEq, Eq)]
pub struct Ran {
    /// Its exit status (`None` when the server sent none).
    pub status: Option<u32>,
}

/// Runs one command on the server and hands every line of its output (stdout and stderr together) to
/// `on_line`, as it arrives. Only the output reaches `on_line`: the command text is never reported.
///
/// # Errors
/// [`FetchError`] (the login or the host key), or [`FetchError::Remote`] when the command cannot be
/// started or the server goes silent for too long.
pub async fn run(
    source: Source,
    command: &str,
    mut on_line: impl FnMut(&str) + Send,
) -> Result<Ran, FetchError> {
    let Source {
        host,
        port,
        user,
        credential,
        host_key,
        allow_local,
        ..
    } = source;
    let (session, _) = open(&host, port, &user, credential, host_key, allow_local).await?;
    let remote = |e: &dyn std::fmt::Display| FetchError::Remote(e.to_string());
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| remote(&e))?;
    channel.exec(true, command).await.map_err(|e| remote(&e))?;
    let mut pending = Vec::<u8>::new();
    let mut status = None;
    let flush = |pending: &mut Vec<u8>, on_line: &mut dyn FnMut(&str), all: bool| {
        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            on_line(String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r'));
        }
        if all && !pending.is_empty() {
            let line = std::mem::take(pending);
            on_line(&String::from_utf8_lossy(&line));
        }
    };
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => {
                pending.extend_from_slice(&data);
                flush(&mut pending, &mut on_line, false);
            }
            russh::ChannelMsg::ExtendedData { data, .. } => {
                pending.extend_from_slice(&data);
                flush(&mut pending, &mut on_line, false);
            }
            russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
            _ => {}
        }
    }
    flush(&mut pending, &mut on_line, true);
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    Ok(Ran { status })
}

/// Copies the file at `source.path` to `dest`. `progress` is told how many bytes are saved,
/// now and then.
///
/// # Errors
/// [`FetchError`]. A file that was begun is removed when the fetch fails.
pub async fn fetch(
    source: Source,
    dest: &Path,
    max_bytes: u64,
    mut progress: impl FnMut(u64) + Send,
) -> Result<Fetched, FetchError> {
    let Source {
        host,
        port,
        user,
        credential,
        host_key,
        path,
        allow_local,
    } = source;
    let (session, host_key) = open(&host, port, &user, credential, host_key, allow_local).await?;
    let remote = |e: &dyn std::fmt::Display| FetchError::Remote(e.to_string());
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| remote(&e))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| remote(&e))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| remote(&e))?;
    let size = sftp
        .metadata(path.clone())
        .await
        .map_err(|e| remote(&e))?
        .size
        .unwrap_or(0);
    if size > max_bytes {
        return Err(FetchError::TooBig(size));
    }
    let mut from = sftp.open(path).await.map_err(|e| remote(&e))?;
    let mut to = tokio::fs::File::create(dest)
        .await
        .map_err(|e| FetchError::Local(e.to_string()))?;
    let result = async {
        let mut saved = 0u64;
        let mut buf = vec![0u8; BLOCK];
        let mut last = 0u64;
        loop {
            let n = from.read(&mut buf).await.map_err(|e| remote(&e))?;
            if n == 0 {
                break;
            }
            saved += n as u64;
            if saved > max_bytes {
                return Err(FetchError::TooBig(saved));
            }
            to.write_all(&buf[..n])
                .await
                .map_err(|e| FetchError::Local(e.to_string()))?;
            if saved - last >= 32 * 1024 * 1024 {
                progress(saved);
                last = saved;
            }
        }
        to.flush()
            .await
            .map_err(|e| FetchError::Local(e.to_string()))?;
        Ok(saved)
    }
    .await;
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    match result {
        Ok(bytes) => Ok(Fetched { bytes, host_key }),
        Err(e) => {
            let _ = tokio::fs::remove_file(dest).await;
            Err(e)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_server_addresses_are_allowed() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(allowed(ip("203.0.113.5"), false));
        assert!(
            allowed(ip("10.1.2.3"), false),
            "a private network is a server network"
        );
        assert!(allowed(ip("2001:db8::1"), false));
        for no in [
            "127.0.0.1",
            "0.0.0.0",
            "169.254.169.254",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            assert!(!allowed(ip(no), false), "{no}");
        }
        assert!(allowed(ip("127.0.0.1"), true), "tests may use this machine");
    }

    #[test]
    fn errors_never_hold_a_credential() {
        let text = FetchError::Login.to_string();
        assert_eq!(text, "the server refused the login");
        let text = FetchError::UnknownHostKey("SHA256:abc".into()).to_string();
        assert!(text.contains("SHA256:abc") && text.contains("host_key_sha256"));
    }
}
