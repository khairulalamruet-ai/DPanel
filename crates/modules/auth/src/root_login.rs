//! Logging in as the server's `root` with the server's own root password (like WHM).
//!
//! The administrator account named `root` has no panel password: its `password_hash` is
//! [`accounts::SYSTEM_ROOT_PASSWORD`]. The control plane never runs as root and cannot read
//! `/etc/shadow`, so the password is handed to `builtpanel-root-auth`, a small root service that
//! answers one question over a Unix socket only the panel's user can open: "is this root's
//! password?". It checks one guess at a time and waits a second after a wrong one, on top of the
//! lockout here.
//!
//! Protocol (one request per connection): the client writes the password's length as two bytes
//! (big endian, at most [`MAX_PASSWORD_BYTES`](crate::MAX_PASSWORD_BYTES)) and then the password;
//! the helper answers one byte: `Y` it is root's password, `N` it is not, `L` root has no usable
//! password (locked or empty: set one with `passwd`), `U` root's password uses a scheme the
//! helper does not know, `E` the helper could not check (e.g. cannot read the file).
//!
//! [`accounts::SYSTEM_ROOT_PASSWORD`]: builtpanel_database::accounts::SYSTEM_ROOT_PASSWORD

use std::{path::PathBuf, sync::Arc, time::Duration};

use secrecy::{ExposeSecret as _, SecretString};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};

/// How long one check may take in all (the helper waits a second after a wrong password and
/// handles one check at a time).
const TIMEOUT: Duration = Duration::from_secs(15);

/// The answer of the root helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootCheck {
    /// The password is root's.
    Match,
    /// It is not.
    Mismatch,
    /// Root has no usable password on this server (locked or empty).
    NoPassword,
    /// Root's password uses a hashing scheme the helper does not know.
    UnsupportedScheme,
}

/// Why the helper could not be asked.
#[derive(Debug, thiserror::Error)]
pub enum RootCheckError {
    /// The socket could not be opened or the exchange failed.
    #[error("the root login helper is not reachable: {0}")]
    Io(#[from] std::io::Error),
    /// No answer in time.
    #[error("the root login helper did not answer in time")]
    Timeout,
    /// The helper answered `E` or something unknown.
    #[error("the root login helper could not check the password")]
    Helper,
}

/// Client of the `builtpanel-root-auth` helper.
#[derive(Debug, Clone)]
pub struct RootLogin {
    socket: Arc<PathBuf>,
}

impl RootLogin {
    /// A client for the helper listening on `socket`.
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket: Arc::new(socket),
        }
    }

    /// Asks whether `password` is the server's root password.
    ///
    /// # Errors
    /// [`RootCheckError`] when the helper cannot be asked.
    pub async fn check(&self, password: &SecretString) -> Result<RootCheck, RootCheckError> {
        tokio::time::timeout(TIMEOUT, self.exchange(password))
            .await
            .map_err(|_| RootCheckError::Timeout)?
    }

    async fn exchange(&self, password: &SecretString) -> Result<RootCheck, RootCheckError> {
        let bytes = password.expose_secret().as_bytes();
        let len = u16::try_from(bytes.len())
            .ok()
            .filter(|&n| usize::from(n) <= crate::MAX_PASSWORD_BYTES)
            .ok_or(RootCheckError::Helper)?;
        let mut stream = UnixStream::connect(self.socket.as_path()).await?;
        let mut message = Vec::with_capacity(2 + bytes.len());
        message.extend_from_slice(&len.to_be_bytes());
        message.extend_from_slice(bytes);
        let sent = stream.write_all(&message).await;
        message.fill(0);
        sent?;
        let mut answer = [0u8; 1];
        stream.read_exact(&mut answer).await?;
        match answer[0] {
            b'Y' => Ok(RootCheck::Match),
            b'N' => Ok(RootCheck::Mismatch),
            b'L' => Ok(RootCheck::NoPassword),
            b'U' => Ok(RootCheck::UnsupportedScheme),
            _ => Err(RootCheckError::Helper),
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use tokio::net::UnixListener;

    use super::*;

    async fn helper_answering(answer: u8) -> (RootLogin, tokio::task::JoinHandle<Vec<u8>>) {
        let dir = std::env::temp_dir().join(format!(
            "bp-root-login-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s");
        let listener = UnixListener::bind(&path).unwrap();
        let task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut len = [0u8; 2];
            s.read_exact(&mut len).await.unwrap();
            let mut pw = vec![0u8; usize::from(u16::from_be_bytes(len))];
            s.read_exact(&mut pw).await.unwrap();
            s.write_all(&[answer]).await.unwrap();
            pw
        });
        (RootLogin::new(path), task)
    }

    #[tokio::test]
    async fn the_password_is_sent_with_its_length_and_the_answer_is_read() {
        for (byte, want) in [
            (b'Y', RootCheck::Match),
            (b'N', RootCheck::Mismatch),
            (b'L', RootCheck::NoPassword),
            (b'U', RootCheck::UnsupportedScheme),
        ] {
            let (client, task) = helper_answering(byte).await;
            let got = client
                .check(&SecretString::from("s3cret pässword"))
                .await
                .unwrap();
            assert_eq!(got, want);
            assert_eq!(task.await.unwrap(), "s3cret pässword".as_bytes());
        }
    }

    #[tokio::test]
    async fn an_unknown_answer_or_a_missing_helper_is_an_error() {
        let (client, _task) = helper_answering(b'E').await;
        assert!(matches!(
            client.check(&SecretString::from("x")).await,
            Err(RootCheckError::Helper)
        ));
        let missing = RootLogin::new(PathBuf::from("/nonexistent/builtpanel/root-auth.sock"));
        assert!(matches!(
            missing.check(&SecretString::from("x")).await,
            Err(RootCheckError::Io(_))
        ));
    }

    #[tokio::test]
    async fn an_oversized_password_is_not_sent() {
        let client = RootLogin::new(PathBuf::from("/nonexistent"));
        let long = SecretString::from("a".repeat(crate::MAX_PASSWORD_BYTES + 1));
        assert!(matches!(
            client.check(&long).await,
            Err(RootCheckError::Helper)
        ));
    }
}
