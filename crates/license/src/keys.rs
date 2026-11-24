//! Signing keys (T17.03): Ed25519, a key id (`kid`), and a provider trait so that the private
//! key can live in a file in development and in a KMS or HSM in production.
//!
//! A [`SigningKey`] only ever hands out signatures and its public half. Nothing here puts a
//! private key into a database, a log or a `Debug` print.

use std::{collections::BTreeMap, io::Write as _, os::unix::fs::OpenOptionsExt as _, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    rand::SystemRandom,
    signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey},
};

/// Length of an Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;
/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// What went wrong with a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The key file could not be read or written.
    #[error("key file: {0}")]
    File(String),
    /// The key is not a valid Ed25519 key.
    #[error("not a valid Ed25519 key")]
    Invalid,
    /// The system could not make a key.
    #[error("could not generate a key")]
    Generate,
    /// A key id that is empty, too long or not plain.
    #[error("a key id is 1 to 64 letters, digits, - and _")]
    KeyId,
}

fn check_kid(kid: &str) -> Result<(), KeyError> {
    if (1..=64).contains(&kid.len())
        && kid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    {
        Ok(())
    } else {
        Err(KeyError::KeyId)
    }
}

/// Something that signs on behalf of a key id. The production implementation asks a KMS or
/// HSM; [`FileSigningKey`] reads a file.
pub trait SigningKey: Send + Sync {
    /// The key id written into what is signed.
    fn kid(&self) -> &str;
    /// The public key (32 bytes).
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN];
    /// Signs `message`.
    ///
    /// # Errors
    /// [`KeyError`] when the key cannot be used.
    fn sign(&self, message: &[u8]) -> Result<[u8; SIGNATURE_LEN], KeyError>;
}

/// A key kept in a file (development, tests). The file holds the PKCS#8 key in base64 and is
/// made readable by its owner only.
pub struct FileSigningKey {
    kid: String,
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for FileSigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSigningKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl FileSigningKey {
    /// Makes a new key and writes it to `path` (mode 0600, never overwriting a file).
    ///
    /// # Errors
    /// [`KeyError`].
    pub fn generate_to_file(path: &Path, kid: &str) -> Result<Self, KeyError> {
        check_kid(kid)?;
        let pkcs8 =
            Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| KeyError::Generate)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| KeyError::File(e.kind().to_string()))?;
        file.write_all(STANDARD.encode(pkcs8.as_ref()).as_bytes())
            .map_err(|e| KeyError::File(e.kind().to_string()))?;
        Self::from_pkcs8(pkcs8.as_ref(), kid)
    }

    /// Reads a key written by [`Self::generate_to_file`]. A file that other users can read
    /// is refused.
    ///
    /// # Errors
    /// [`KeyError`].
    pub fn load(path: &Path, kid: &str) -> Result<Self, KeyError> {
        use std::os::unix::fs::PermissionsExt as _;
        check_kid(kid)?;
        let mode = std::fs::metadata(path)
            .map_err(|e| KeyError::File(e.kind().to_string()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(KeyError::File("the key file is readable by others".into()));
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| KeyError::File(e.kind().to_string()))?;
        let der = STANDARD
            .decode(text.trim())
            .map_err(|_| KeyError::Invalid)?;
        Self::from_pkcs8(&der, kid)
    }

    fn from_pkcs8(der: &[u8], kid: &str) -> Result<Self, KeyError> {
        Ok(Self {
            kid: kid.to_owned(),
            pair: Ed25519KeyPair::from_pkcs8(der).map_err(|_| KeyError::Invalid)?,
        })
    }
}

impl SigningKey for FileSigningKey {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        let mut out = [0u8; PUBLIC_KEY_LEN];
        out.copy_from_slice(self.pair.public_key().as_ref());
        out
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; SIGNATURE_LEN], KeyError> {
        let mut out = [0u8; SIGNATURE_LEN];
        out.copy_from_slice(self.pair.sign(message).as_ref());
        Ok(out)
    }
}

/// Whether `signature` is `public_key`'s signature of `message` (a key that is not on a list:
/// an installation's own, which proves it holds the private half).
#[must_use]
pub fn verify_with(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(message, signature)
        .is_ok()
}

/// The public keys a verifier trusts, by key id (the bundle that ships with Built Panel, so a
/// rotated key can be added without a rebuild of anything that signs).
#[derive(Debug, Clone, Default)]
pub struct PublicKeys {
    keys: BTreeMap<String, [u8; PUBLIC_KEY_LEN]>,
}

impl PublicKeys {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a key.
    ///
    /// # Errors
    /// [`KeyError::KeyId`].
    pub fn with(mut self, kid: &str, key: [u8; PUBLIC_KEY_LEN]) -> Result<Self, KeyError> {
        check_kid(kid)?;
        self.keys.insert(kid.to_owned(), key);
        Ok(self)
    }

    /// Reads a set written as lines of `kid base64-public-key`.
    ///
    /// # Errors
    /// [`KeyError`].
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let mut set = Self::new();
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (kid, key) = line
                .split_once(char::is_whitespace)
                .ok_or(KeyError::Invalid)?;
            let bytes = STANDARD.decode(key.trim()).map_err(|_| KeyError::Invalid)?;
            let key: [u8; PUBLIC_KEY_LEN] = bytes.try_into().map_err(|_| KeyError::Invalid)?;
            set = set.with(kid, key)?;
        }
        Ok(set)
    }

    /// The text [`Self::parse`] reads.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.keys
            .iter()
            .map(|(kid, key)| format!("{kid} {}\n", STANDARD.encode(key)))
            .collect()
    }

    /// The public key of `kid`.
    #[must_use]
    pub fn get(&self, kid: &str) -> Option<&[u8; PUBLIC_KEY_LEN]> {
        self.keys.get(kid)
    }

    /// Whether `signature` is `kid`'s signature of `message`.
    #[must_use]
    pub fn verify(&self, kid: &str, message: &[u8], signature: &[u8]) -> bool {
        self.get(kid).is_some_and(|key| {
            UnparsedPublicKey::new(&ED25519, key)
                .verify(message, signature)
                .is_ok()
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bp-license-keys-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_key_signs_and_only_its_own_public_key_verifies() {
        let d = dir();
        let key = FileSigningKey::generate_to_file(&d.join("a.key"), "lic-2026-01").unwrap();
        let other = FileSigningKey::generate_to_file(&d.join("b.key"), "lic-2026-02").unwrap();
        let set = PublicKeys::new()
            .with(key.kid(), key.public_key())
            .unwrap()
            .with(other.kid(), other.public_key())
            .unwrap();
        let sig = key.sign(b"hello").unwrap();
        assert!(set.verify("lic-2026-01", b"hello", &sig));
        assert!(
            !set.verify("lic-2026-01", b"hello!", &sig),
            "another message"
        );
        assert!(!set.verify("lic-2026-02", b"hello", &sig), "another key");
        assert!(!set.verify("unknown", b"hello", &sig), "an unknown key id");
        assert!(
            !set.verify("lic-2026-01", b"hello", &sig[..63]),
            "a short signature"
        );
        // The set round trips through its text form.
        let again = PublicKeys::parse(&set.to_text()).unwrap();
        assert!(again.verify("lic-2026-02", b"x", &other.sign(b"x").unwrap()));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_key_file_is_private_and_is_never_overwritten_or_read_when_others_can_read_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = dir();
        let path = d.join("k.key");
        let key = FileSigningKey::generate_to_file(&path, "k1").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            FileSigningKey::generate_to_file(&path, "k1").is_err(),
            "never overwritten"
        );
        let loaded = FileSigningKey::load(&path, "k1").unwrap();
        assert_eq!(loaded.public_key(), key.public_key());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            FileSigningKey::load(&path, "k1"),
            Err(KeyError::File(_))
        ));
        assert!(FileSigningKey::generate_to_file(&d.join("x"), "bad kid!").is_err());
        // Debug never shows the key.
        assert!(!format!("{key:?}").contains(&STANDARD.encode(key.public_key())));
        let _ = std::fs::remove_dir_all(d);
    }
}
