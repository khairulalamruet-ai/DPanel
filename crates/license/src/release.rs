//! The release manifest (T17.14): which files make a Built Panel release, with their sizes and
//! SHA-256 hashes, signed with the release key (a key of its own, never the license key). An
//! installation checks the signature and then every file against it before it installs
//! anything (T17.15).

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{CanonicalError, canonical_json},
    hash::sha256,
    keys::{PublicKeys, SigningKey},
};

/// The manifest's schema version.
pub const MANIFEST_VERSION: u32 = 1;

/// What is put in front of the manifest before it is signed, so that a signature made for
/// anything else (an entitlement, say) can never pass as a release manifest.
const RELEASE_DOMAIN: &[u8] = b"builtpanel.release.v1\n";

/// The most files a manifest may list.
pub const MAX_COMPONENTS: usize = 256;

/// One file of a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// The file's name (no folders), such as `builtpanel-control-api`.
    pub name: String,
    /// Its SHA-256, as 64 lower-case hexadecimal digits.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// What a release is made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    /// [`MANIFEST_VERSION`].
    pub v: u32,
    /// The release's version, such as `0.9.0`.
    pub version: String,
    /// The source commit it was built from.
    pub commit: String,
    /// When it was made (Unix seconds).
    pub created_at: i64,
    /// The files, ordered by name.
    pub components: Vec<Component>,
}

/// A manifest and its signature, as they travel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedManifest {
    /// The release key's id.
    pub kid: String,
    /// `EdDSA`.
    pub alg: String,
    /// The canonical manifest, base64url.
    pub payload: String,
    /// The signature, base64url.
    pub signature: String,
}

/// What can go wrong.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReleaseError {
    /// The manifest is not well formed (bad names, hashes, duplicates, too many files).
    #[error("the release manifest is not valid: {0}")]
    Invalid(&'static str),
    /// The document cannot be read, or is not the canonical form.
    #[error("the release manifest is malformed")]
    Malformed,
    /// The signature does not match, or the key is not trusted.
    #[error("the release manifest's signature is not good")]
    Signature,
    /// The key could not sign.
    #[error("the release key could not sign")]
    Sign,
    /// A file is not in the manifest.
    #[error("{0} is not part of this release")]
    Unknown(String),
    /// A file is not what the manifest says.
    #[error("{0} does not match the release manifest")]
    Mismatch(String),
}

impl From<CanonicalError> for ReleaseError {
    fn from(_: CanonicalError) -> Self {
        Self::Malformed
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

impl ReleaseManifest {
    /// Builds a manifest from files already read (name and content); the order does not matter.
    ///
    /// # Errors
    /// [`ReleaseError::Invalid`].
    pub fn build(
        version: &str,
        commit: &str,
        created_at: i64,
        files: &[(String, Vec<u8>)],
    ) -> Result<Self, ReleaseError> {
        let mut components: Vec<Component> = files
            .iter()
            .map(|(name, bytes)| Component {
                name: name.clone(),
                sha256: hex(&sha256(bytes)),
                size: bytes.len() as u64,
            })
            .collect();
        components.sort_by(|a, b| a.name.cmp(&b.name));
        let manifest = Self {
            v: MANIFEST_VERSION,
            version: version.to_owned(),
            commit: commit.to_owned(),
            created_at,
            components,
        };
        manifest.check()?;
        Ok(manifest)
    }

    /// Checks the manifest is well formed.
    ///
    /// # Errors
    /// [`ReleaseError::Invalid`].
    pub fn check(&self) -> Result<(), ReleaseError> {
        if self.v != MANIFEST_VERSION {
            return Err(ReleaseError::Invalid("unknown schema version"));
        }
        if self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-+.".contains(c))
        {
            return Err(ReleaseError::Invalid("bad version"));
        }
        if self.commit.len() > 64 || !self.commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ReleaseError::Invalid("bad commit"));
        }
        if self.components.is_empty() || self.components.len() > MAX_COMPONENTS {
            return Err(ReleaseError::Invalid("bad number of files"));
        }
        for pair in self.components.windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(ReleaseError::Invalid("files are not ordered and unique"));
            }
        }
        for c in &self.components {
            if !valid_name(&c.name) {
                return Err(ReleaseError::Invalid("bad file name"));
            }
            if c.sha256.len() != 64 || !c.sha256.chars().all(|d| matches!(d, '0'..='9' | 'a'..='f'))
            {
                return Err(ReleaseError::Invalid("bad hash"));
            }
        }
        Ok(())
    }

    /// Checks `bytes` is the file `name` of this release (size and hash).
    ///
    /// # Errors
    /// [`ReleaseError::Unknown`] or [`ReleaseError::Mismatch`].
    pub fn check_file(&self, name: &str, bytes: &[u8]) -> Result<(), ReleaseError> {
        let component = self
            .components
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| ReleaseError::Unknown(name.to_owned()))?;
        if component.size != bytes.len() as u64 || component.sha256 != hex(&sha256(bytes)) {
            return Err(ReleaseError::Mismatch(name.to_owned()));
        }
        Ok(())
    }
}

/// The text an integrity report is signed over (by the installation, under the scan's nonce):
/// the scan's id and then `name=sha256` for each file, in name order. Both sides write it
/// with this function.
#[must_use]
pub fn integrity_report_text(
    scan_id: &str,
    files: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut text = scan_id.to_owned();
    for (name, hash) in files {
        text.push('\n');
        text.push_str(name);
        text.push('=');
        text.push_str(hash);
    }
    text
}

/// The SHA-256 of `bytes` as the hex a manifest and a report use.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn signed_bytes(payload: &[u8]) -> Vec<u8> {
    let mut m = RELEASE_DOMAIN.to_vec();
    m.extend_from_slice(payload);
    m
}

impl SignedManifest {
    /// Signs a manifest.
    ///
    /// # Errors
    /// [`ReleaseError`].
    pub fn sign(manifest: &ReleaseManifest, key: &dyn SigningKey) -> Result<Self, ReleaseError> {
        manifest.check()?;
        let value = serde_json::to_value(manifest).map_err(|_| ReleaseError::Malformed)?;
        let payload = canonical_json(&value)?;
        let signature = key
            .sign(&signed_bytes(&payload))
            .map_err(|_| ReleaseError::Sign)?;
        Ok(Self {
            kid: key.kid().to_owned(),
            alg: "EdDSA".to_owned(),
            payload: URL_SAFE_NO_PAD.encode(&payload),
            signature: URL_SAFE_NO_PAD.encode(signature),
        })
    }

    /// Checks the signature (with a trusted release key) and that what was signed is the
    /// canonical form of a good manifest; returns the manifest.
    ///
    /// # Errors
    /// [`ReleaseError`].
    pub fn verify(&self, keys: &PublicKeys) -> Result<ReleaseManifest, ReleaseError> {
        if self.alg != "EdDSA" {
            return Err(ReleaseError::Malformed);
        }
        let payload = URL_SAFE_NO_PAD
            .decode(&self.payload)
            .map_err(|_| ReleaseError::Malformed)?;
        let signature = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| ReleaseError::Malformed)?;
        if !keys.verify(&self.kid, &signed_bytes(&payload), &signature) {
            return Err(ReleaseError::Signature);
        }
        let manifest: ReleaseManifest =
            serde_json::from_slice(&payload).map_err(|_| ReleaseError::Malformed)?;
        let value = serde_json::to_value(&manifest).map_err(|_| ReleaseError::Malformed)?;
        if canonical_json(&value)? != payload {
            return Err(ReleaseError::Malformed);
        }
        manifest.check()?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::keys::FileSigningKey;

    fn key(dir: &std::path::Path, kid: &str) -> FileSigningKey {
        FileSigningKey::generate_to_file(&dir.join(format!("{kid}.key")), kid).unwrap()
    }

    fn files() -> Vec<(String, Vec<u8>)> {
        vec![
            ("builtpanel-worker".into(), b"worker binary".to_vec()),
            ("builtpanel-control-api".into(), b"api binary".to_vec()),
        ]
    }

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bp-release-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_signed_manifest_verifies_and_lists_the_files_in_order() {
        let dir = tmp();
        let k = key(&dir, "release-1");
        let m = ReleaseManifest::build("1.2.3", "abc123", 1_700_000_000, &files()).unwrap();
        assert_eq!(m.components[0].name, "builtpanel-control-api");
        let signed = SignedManifest::sign(&m, &k).unwrap();
        let keys = PublicKeys::new().with("release-1", k.public_key()).unwrap();
        assert_eq!(signed.verify(&keys).unwrap(), m);
        // The same manifest always gives the same bytes.
        assert_eq!(
            signed.payload,
            SignedManifest::sign(&m, &k).unwrap().payload
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_byte_a_wrong_key_and_a_license_signature_are_all_refused() {
        let dir = tmp();
        let k = key(&dir, "release-1");
        let other = key(&dir, "release-2");
        let m = ReleaseManifest::build("1.2.3", "abc123", 1, &files()).unwrap();
        let signed = SignedManifest::sign(&m, &k).unwrap();
        let keys = PublicKeys::new().with("release-1", k.public_key()).unwrap();
        // One byte of the payload changed.
        let mut bad = signed.clone();
        let mut raw = URL_SAFE_NO_PAD.decode(&bad.payload).unwrap();
        raw[10] ^= 1;
        bad.payload = URL_SAFE_NO_PAD.encode(raw);
        assert!(bad.verify(&keys).is_err());
        // Signed by a key that is not trusted, or claiming another key's id.
        let by_other = SignedManifest::sign(&m, &other).unwrap();
        assert_eq!(by_other.verify(&keys), Err(ReleaseError::Signature));
        let mut lying = by_other;
        lying.kid = "release-1".into();
        assert_eq!(lying.verify(&keys), Err(ReleaseError::Signature));
        // A signature made for the payload without the release domain is no release signature.
        let value = serde_json::to_value(&m).unwrap();
        let payload = canonical_json(&value).unwrap();
        let plain = k.sign(&payload).unwrap();
        let forged = SignedManifest {
            kid: "release-1".into(),
            alg: "EdDSA".into(),
            payload: URL_SAFE_NO_PAD.encode(&payload),
            signature: URL_SAFE_NO_PAD.encode(plain),
        };
        assert_eq!(forged.verify(&keys), Err(ReleaseError::Signature));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_is_checked_against_its_hash_and_size() {
        let m = ReleaseManifest::build("1.0.0", "abc", 1, &files()).unwrap();
        m.check_file("builtpanel-worker", b"worker binary").unwrap();
        assert_eq!(
            m.check_file("builtpanel-worker", b"worker binarx"),
            Err(ReleaseError::Mismatch("builtpanel-worker".into()))
        );
        assert_eq!(
            m.check_file("builtpanel-worker", b"worker binary!"),
            Err(ReleaseError::Mismatch("builtpanel-worker".into()))
        );
        assert_eq!(
            m.check_file("other", b"x"),
            Err(ReleaseError::Unknown("other".into()))
        );
    }

    #[test]
    fn a_bad_manifest_is_not_made_or_accepted() {
        let mut dup = files();
        dup.push(("builtpanel-worker".into(), b"x".to_vec()));
        assert!(ReleaseManifest::build("1.0.0", "abc", 1, &dup).is_err());
        assert!(ReleaseManifest::build("1.0.0", "abc", 1, &[("../x".into(), vec![1])]).is_err());
        assert!(ReleaseManifest::build("1.0.0", "abc", 1, &[]).is_err());
        assert!(ReleaseManifest::build("", "abc", 1, &files()).is_err());
        assert!(ReleaseManifest::build("1.0.0", "not-hex!", 1, &files()).is_err());
    }
}
