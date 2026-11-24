//! The installation's identity (T17.07): an id and a key pair made once, at first start, and
//! kept in a private folder. The folder is part of the system backup, so a reinstall from a
//! backup is the same installation and does not use up another activation.

use std::{io::Write as _, os::unix::fs::OpenOptionsExt as _, path::Path};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use builtpanel_license::{FileSigningKey, KeyError, SigningKey as _};
use uuid::Uuid;

/// The file with the installation's id.
pub const ID_FILE: &str = "installation.id";
/// The file with its private key.
pub const KEY_FILE: &str = "installation.key";

/// Why the identity could not be had.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// A file could not be read or written.
    #[error("identity files: {0}")]
    File(String),
    /// The id file does not hold an id, or the key is not usable.
    #[error("the identity files are damaged")]
    Damaged,
}

/// Who this installation is.
pub struct Identity {
    /// Its id.
    pub installation_id: Uuid,
    key: FileSigningKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("installation_id", &self.installation_id)
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Reads the identity in `dir`, or makes it (the folder private, the files 0600) when
    /// there is none yet. Half an identity (one file) is an error, never silently replaced.
    ///
    /// # Errors
    /// [`IdentityError`].
    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        use std::os::unix::fs::DirBuilderExt as _;
        let io = |e: std::io::Error| IdentityError::File(e.kind().to_string());
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(io)?;
        let (id_path, key_path) = (dir.join(ID_FILE), dir.join(KEY_FILE));
        match (id_path.exists(), key_path.exists()) {
            (true, true) => {
                let id = std::fs::read_to_string(&id_path)
                    .map_err(io)?
                    .trim()
                    .parse::<Uuid>()
                    .map_err(|_| IdentityError::Damaged)?;
                let key = FileSigningKey::load(&key_path, "installation").map_err(|e| match e {
                    KeyError::File(f) => IdentityError::File(f),
                    _ => IdentityError::Damaged,
                })?;
                Ok(Self {
                    installation_id: id,
                    key,
                })
            }
            (false, false) => {
                let id = Uuid::new_v4();
                let key = FileSigningKey::generate_to_file(&key_path, "installation")
                    .map_err(|e| IdentityError::File(e.to_string()))?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&id_path)
                    .map_err(io)?;
                file.write_all(id.to_string().as_bytes()).map_err(io)?;
                Ok(Self {
                    installation_id: id,
                    key,
                })
            }
            _ => Err(IdentityError::Damaged),
        }
    }

    /// The public key, base64, as the authority registers it.
    #[must_use]
    pub fn public_key(&self) -> String {
        STANDARD.encode(self.key.public_key())
    }

    /// Signs the proof an authority asks for (see the authority's `proof_message`).
    ///
    /// # Errors
    /// [`IdentityError::Damaged`] when the key cannot sign.
    pub fn prove(
        &self,
        purpose: &str,
        nonce: &str,
        extra: Option<&str>,
    ) -> Result<String, IdentityError> {
        let mut m = format!("builtpanel-{purpose}-v1\n{nonce}\n{}", self.installation_id);
        if let Some(extra) = extra {
            m.push('\n');
            m.push_str(extra);
        }
        let sig = self
            .key
            .sign(m.as_bytes())
            .map_err(|_| IdentityError::Damaged)?;
        Ok(URL_SAFE_NO_PAD.encode(sig))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("bp-identity-{}", Uuid::new_v4().simple()))
    }

    #[test]
    fn the_identity_is_made_once_and_is_the_same_every_time_after() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = dir();
        let first = Identity::load_or_create(&d).unwrap();
        let again = Identity::load_or_create(&d).unwrap();
        assert_eq!(first.installation_id, again.installation_id);
        assert_eq!(first.public_key(), again.public_key());
        for (file, mode) in [(ID_FILE, 0o600), (KEY_FILE, 0o600)] {
            assert_eq!(
                std::fs::metadata(d.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                mode,
                "{file}"
            );
        }
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // Copied to a new machine (restored from a backup), it is the same installation.
        let restored = dir();
        std::fs::create_dir_all(&restored).unwrap();
        for file in [ID_FILE, KEY_FILE] {
            std::fs::copy(d.join(file), restored.join(file)).unwrap();
            std::fs::set_permissions(restored.join(file), std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let again = Identity::load_or_create(&restored).unwrap();
        assert_eq!(again.installation_id, first.installation_id);
        assert_eq!(
            again.prove("refresh", "n", None).unwrap(),
            first.prove("refresh", "n", None).unwrap()
        );
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(restored);
    }

    #[test]
    fn half_an_identity_is_not_replaced() {
        let d = dir();
        Identity::load_or_create(&d).unwrap();
        std::fs::remove_file(d.join(KEY_FILE)).unwrap();
        assert!(matches!(
            Identity::load_or_create(&d),
            Err(IdentityError::Damaged)
        ));
        std::fs::write(d.join(KEY_FILE), "x").unwrap();
        let _ = std::fs::remove_dir_all(d);
    }
}
