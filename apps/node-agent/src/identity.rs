//! The agent's identity on disk.
//!
//! ```text
//! /var/lib/builtpanel-agent/        0700 (nothing for group/others), owned by builtpanel-agent
//!   node.key        0600  private key; created on this machine, never leaves it
//!   node.crt        0644  node certificate
//!   ca.crt          0644  CA certificate (pinned during enrolment)
//!   identity.json   0644  node id, name, server, CA fingerprint, expiry (no secrets)
//! ```
//!
//! `identity.json` is written last: its presence means the other three files are
//! complete. Existing files are never overwritten unless re-enrolment is asked
//! for explicitly, and a failure part-way removes what was written.

use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where the identity lives unless told otherwise.
pub const DEFAULT_STATE_DIR: &str = "/var/lib/builtpanel-agent";
/// Private key file name.
pub const KEY_FILE: &str = "node.key";
/// Node certificate file name.
pub const CERT_FILE: &str = "node.crt";
/// CA certificate file name.
pub const CA_FILE: &str = "ca.crt";
/// Identity file name.
pub const IDENTITY_FILE: &str = "identity.json";

/// Everything about the node except secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The node's id at the control plane.
    pub node_id: Uuid,
    /// The node's name.
    pub name: String,
    /// The control plane address it enrolled with.
    pub server: String,
    /// SHA-256 fingerprint of the pinned CA (lowercase hex).
    pub ca_fingerprint_sha256: String,
    /// When it enrolled (RFC 3339, UTC).
    pub enrolled_at: String,
    /// When its certificate expires (Unix seconds).
    pub certificate_not_after: i64,
    /// Protocol version announced at enrolment.
    pub protocol_version: u16,
    /// Where the agent opens its long-lived connection (`https://host:port`).
    /// Absent when the control plane did not say; `run --stream-url` supplies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
}

/// What enrolment hands to [`StateDir::save`].
pub struct Material<'a> {
    /// The private key (PEM).
    pub key_pem: &'a SecretString,
    /// The node certificate (PEM).
    pub cert_pem: &'a str,
    /// The CA certificate (PEM).
    pub ca_pem: &'a str,
    /// The non-secret facts.
    pub identity: &'a Identity,
}

/// What [`StateDir::load`] returns. The key stays on disk.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The identity.
    pub identity: Identity,
    /// Path of the private key, for the TLS stack to read.
    pub key_path: PathBuf,
    /// Path of the node certificate.
    pub cert_path: PathBuf,
    /// Path of the CA certificate.
    pub ca_path: PathBuf,
}

/// Problems with the stored identity.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// This node already has an identity.
    #[error("{0} already holds an enrolled identity; use --force to replace it")]
    AlreadyEnrolled(PathBuf),
    /// No identity is stored.
    #[error("this node is not enrolled ({0} has no identity)")]
    NotEnrolled(PathBuf),
    /// The state directory is open to others.
    #[error("{path}: permissions {mode:o} let other users in; use 0700 or 0750")]
    InsecureDirectory {
        /// The directory.
        path: PathBuf,
        /// Its mode bits.
        mode: u32,
    },
    /// The private key is readable by others.
    #[error("{path}: permissions {mode:o} are too open; the key must be 0600 or 0400")]
    InsecureKey {
        /// The key file.
        path: PathBuf,
        /// Its mode bits.
        mode: u32,
    },
    /// `identity.json` is unreadable.
    #[error("{0}: not a valid identity file")]
    Corrupt(PathBuf),
    /// File system error.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StateError + '_ {
    move |source| StateError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Result<u32, StateError> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::metadata(path)
        .map_err(io_err(path))?
        .permissions()
        .mode()
        & 0o777)
}

fn write_new(path: &Path, contents: &str, mode: u32) -> Result<(), StateError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    // Only files this call created are ever removed: if `open` fails (for
    // instance because something already exists there) nothing is touched.
    let mut file = options.open(path).map_err(io_err(path))?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(source) = written {
        let _ = fs::remove_file(path);
        return Err(io_err(path)(source));
    }
    Ok(())
}

/// The directory holding the identity.
#[derive(Debug, Clone)]
pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    /// A handle on `root`; nothing is touched yet.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Whether any part of an identity is already stored.
    #[must_use]
    pub fn is_enrolled(&self) -> bool {
        [KEY_FILE, CERT_FILE, CA_FILE, IDENTITY_FILE]
            .iter()
            .any(|name| self.file(name).exists())
    }

    /// Creates the directory (mode 0700) or checks that an existing one does
    /// not let other users in.
    ///
    /// # Errors
    /// [`StateError::InsecureDirectory`] or file system errors.
    pub fn prepare(&self) -> Result<(), StateError> {
        if self.root.exists() {
            #[cfg(unix)]
            {
                let mode = mode_of(&self.root)?;
                if mode & 0o007 != 0 {
                    return Err(StateError::InsecureDirectory {
                        path: self.root.clone(),
                        mode,
                    });
                }
            }
            return Ok(());
        }
        fs::create_dir_all(&self.root).map_err(io_err(&self.root))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(io_err(&self.root))?;
        }
        Ok(())
    }

    /// Proves files can be created here, so enrolment can fail early, before
    /// the one-time token is used up.
    ///
    /// # Errors
    /// File system errors.
    pub fn ensure_writable(&self) -> Result<(), StateError> {
        self.prepare()?;
        let probe = self.file(&format!(".write-test-{}", std::process::id()));
        write_new(&probe, "", 0o600)?;
        fs::remove_file(&probe).map_err(io_err(&probe))
    }

    /// Stores a new identity: key (0600) first, `identity.json` last. Refuses
    /// to overwrite an existing identity unless `replace` is set, in which case
    /// the old files are removed first. On failure nothing is left behind.
    ///
    /// # Errors
    /// [`StateError::AlreadyEnrolled`] or file system errors.
    pub fn save(&self, material: &Material<'_>, replace: bool) -> Result<(), StateError> {
        self.prepare()?;
        if self.is_enrolled() {
            if !replace {
                return Err(StateError::AlreadyEnrolled(self.root.clone()));
            }
            for name in [IDENTITY_FILE, CERT_FILE, CA_FILE, KEY_FILE] {
                match fs::remove_file(self.file(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_err(&self.file(name))(e)),
                }
            }
        }
        let identity_json = serde_json::to_string_pretty(material.identity)
            .map_err(|_| StateError::Corrupt(self.file(IDENTITY_FILE)))?;
        let steps: [(&str, &str, u32); 4] = [
            (KEY_FILE, material.key_pem.expose_secret(), 0o600),
            (CERT_FILE, material.cert_pem, 0o644),
            (CA_FILE, material.ca_pem, 0o644),
            (IDENTITY_FILE, &identity_json, 0o644),
        ];
        for (written, (name, contents, mode)) in steps.iter().enumerate() {
            if let Err(err) = write_new(&self.file(name), contents, *mode) {
                // Remove what was written before this step, never the failed
                // step's own path (it may belong to somebody else).
                for (done, _, _) in steps.iter().take(written) {
                    let _ = fs::remove_file(self.file(done));
                }
                return Err(err);
            }
        }
        Ok(())
    }

    /// Reads the stored identity and checks the private key is not open to
    /// others. The key itself is not read.
    ///
    /// # Errors
    /// [`StateError::NotEnrolled`], [`StateError::InsecureKey`],
    /// [`StateError::Corrupt`] or file system errors.
    pub fn load(&self) -> Result<Loaded, StateError> {
        let identity_path = self.file(IDENTITY_FILE);
        if !identity_path.exists() {
            return Err(StateError::NotEnrolled(self.root.clone()));
        }
        let key_path = self.file(KEY_FILE);
        #[cfg(unix)]
        {
            let mode = mode_of(&key_path)?;
            if mode & 0o077 != 0 {
                return Err(StateError::InsecureKey {
                    path: key_path,
                    mode,
                });
            }
        }
        let text = fs::read_to_string(&identity_path).map_err(io_err(&identity_path))?;
        let identity: Identity =
            serde_json::from_str(&text).map_err(|_| StateError::Corrupt(identity_path))?;
        let (cert_path, ca_path) = (self.file(CERT_FILE), self.file(CA_FILE));
        for path in [&cert_path, &ca_path] {
            if !path.exists() {
                return Err(StateError::Corrupt(path.clone()));
            }
        }
        Ok(Loaded {
            identity,
            key_path,
            cert_path,
            ca_path,
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "builtpanel-agent-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn identity() -> Identity {
        Identity {
            node_id: "0190b6a2-0000-7000-8000-000000000001".parse().unwrap(),
            name: "eu-west-1".into(),
            server: "https://panel.example.com".into(),
            ca_fingerprint_sha256: "ab".repeat(32),
            enrolled_at: "2026-11-12T10:00:00Z".into(),
            certificate_not_after: 1_800_000_000,
            protocol_version: 1,
            stream_url: Some("https://panel.example.com:8444".into()),
        }
    }

    fn save(dir: &StateDir, key: &str, replace: bool) -> Result<(), StateError> {
        let key = SecretString::from(key.to_owned());
        dir.save(
            &Material {
                key_pem: &key,
                cert_pem: "CERT",
                ca_pem: "CA",
                identity: &identity(),
            },
            replace,
        )
    }

    #[cfg(unix)]
    #[test]
    fn the_identity_is_stored_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = temp("save");
        let dir = StateDir::new(base.join("state"));
        assert!(!dir.is_enrolled());
        save(&dir, "SECRET-KEY", false).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.path()), 0o700, "a new directory is private");
        assert_eq!(mode(&dir.file(KEY_FILE)), 0o600);
        for public in [CERT_FILE, CA_FILE, IDENTITY_FILE] {
            assert_eq!(mode(&dir.file(public)), 0o644, "{public}");
        }
        assert!(dir.is_enrolled());
        // The key is in exactly one file; the identity file holds no secrets.
        assert_eq!(
            fs::read_to_string(dir.file(KEY_FILE)).unwrap(),
            "SECRET-KEY"
        );
        for name in [CERT_FILE, CA_FILE, IDENTITY_FILE] {
            assert!(
                !fs::read_to_string(dir.file(name))
                    .unwrap()
                    .contains("SECRET"),
                "{name}"
            );
        }
        let loaded = dir.load().unwrap();
        assert_eq!(loaded.identity, identity());
        assert_eq!(loaded.key_path, dir.file(KEY_FILE));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn an_existing_identity_is_never_overwritten_without_asking() {
        let base = temp("overwrite");
        let dir = StateDir::new(&base);
        save(&dir, "KEY-ONE", false).unwrap();
        let err = save(&dir, "KEY-TWO", false).unwrap_err();
        assert!(matches!(err, StateError::AlreadyEnrolled(_)), "{err}");
        assert_eq!(fs::read_to_string(dir.file(KEY_FILE)).unwrap(), "KEY-ONE");
        // Even a lone leftover file blocks it.
        fs::remove_file(dir.file(IDENTITY_FILE)).unwrap();
        assert!(matches!(
            save(&dir, "KEY-TWO", false),
            Err(StateError::AlreadyEnrolled(_))
        ));
        // Replacing is possible when asked for, and is complete.
        save(&dir, "KEY-THREE", true).unwrap();
        assert_eq!(fs::read_to_string(dir.file(KEY_FILE)).unwrap(), "KEY-THREE");
        assert!(dir.load().is_ok());
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_leaves_nothing_half_written() {
        let base = temp("rollback");
        let dir = StateDir::new(&base);
        dir.prepare().unwrap();
        // A dangling symlink is invisible to `exists()` but blocks creating the
        // third file, so the failure happens after two files were written.
        std::os::unix::fs::symlink("/nonexistent/target", dir.file(CA_FILE)).unwrap();
        assert!(!dir.is_enrolled());
        let err = save(&dir, "KEY", false).unwrap_err();
        assert!(
            matches!(&err, StateError::Io { path, .. } if path.ends_with(CA_FILE)),
            "{err}"
        );
        assert!(
            !dir.file(KEY_FILE).exists(),
            "the key written earlier is removed again"
        );
        assert!(!dir.file(CERT_FILE).exists());
        assert!(
            !dir.file(IDENTITY_FILE).exists(),
            "no identity means not enrolled"
        );
        assert!(
            fs::symlink_metadata(dir.file(CA_FILE)).is_ok(),
            "what was already in the way is left alone"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn loose_permissions_are_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = temp("perms");
        let dir = StateDir::new(&base);
        save(&dir, "KEY", false).unwrap();
        // A key others can read.
        fs::set_permissions(dir.file(KEY_FILE), fs::Permissions::from_mode(0o640)).unwrap();
        let err = dir.load().unwrap_err();
        assert!(
            matches!(err, StateError::InsecureKey { mode: 0o640, .. }),
            "{err}"
        );
        assert!(!err.to_string().contains("KEY\n"));
        fs::set_permissions(dir.file(KEY_FILE), fs::Permissions::from_mode(0o400)).unwrap();
        assert!(dir.load().is_ok(), "0400 is fine");
        // A directory open to everyone.
        fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            dir.prepare(),
            Err(StateError::InsecureDirectory { mode: 0o755, .. })
        ));
        fs::set_permissions(&base, fs::Permissions::from_mode(0o750)).unwrap();
        assert!(dir.prepare().is_ok(), "group access (0750) is allowed");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_and_damaged_identities_are_reported() {
        let base = temp("missing");
        let dir = StateDir::new(&base);
        assert!(matches!(dir.load(), Err(StateError::NotEnrolled(_))));
        save(&dir, "KEY", false).unwrap();
        fs::remove_file(dir.file(CA_FILE)).unwrap();
        assert!(matches!(dir.load(), Err(StateError::Corrupt(p)) if p.ends_with(CA_FILE)));
        fs::write(dir.file(CA_FILE), "CA").unwrap();
        fs::write(dir.file(IDENTITY_FILE), "{not json").unwrap();
        assert!(matches!(dir.load(), Err(StateError::Corrupt(_))));
        let _ = fs::remove_dir_all(&base);
    }
}
