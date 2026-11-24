//! Where backups are stored (T09.03).
//!
//! A [`Destination`] stores objects (a backup's parts and manifest) under names like
//! `<backup id>/files.tar.zst.enc`. Three kinds: a folder ([`local`]), an S3-compatible
//! bucket ([`s3`]) and an SFTP folder ([`sftp`]). Everything is blocking and streams: a
//! part of many gigabytes is never held in memory. A destination never sees a key or a
//! plain byte: what it stores is already encrypted.

pub mod local;
pub mod open;
pub mod s3;
pub mod sftp;

use std::io::Read;

/// Why a destination operation failed (said without echoing what a server sent back).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DestError {
    /// The name of an object is not acceptable (folders, `..`, odd characters).
    #[error("the object name is not acceptable")]
    BadName,
    /// The settings or credentials are not usable.
    #[error("the destination is not set up correctly: {0}")]
    Config(String),
    /// The server cannot be reached.
    #[error("the destination cannot be reached: {0}")]
    Unreachable(String),
    /// The server refused the credentials or the operation.
    #[error("the destination refused: {0}")]
    Refused(String),
    /// The server's key is not the pinned one (the fingerprint it showed is given).
    #[error("the server's key is not the pinned one; it shows {0}")]
    HostKey(String),
    /// No such object.
    #[error("not found")]
    NotFound,
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// A place to keep objects.
pub trait Destination: Send {
    /// Writes an object from `data`, replacing one of that name; returns the bytes written.
    ///
    /// # Errors
    /// [`DestError`].
    fn put(&self, name: &str, data: &mut dyn Read) -> Result<u64, DestError>;

    /// Reads an object.
    ///
    /// # Errors
    /// [`DestError::NotFound`] and the others.
    fn get(&self, name: &str) -> Result<Box<dyn Read + Send>, DestError>;

    /// Removes an object (none there is not an error).
    ///
    /// # Errors
    /// [`DestError`].
    fn delete(&self, name: &str) -> Result<(), DestError>;

    /// The names that start with `prefix`.
    ///
    /// # Errors
    /// [`DestError`].
    fn list(&self, prefix: &str) -> Result<Vec<String>, DestError>;

    /// Proves the destination works: writes a small object, reads it back, removes it.
    ///
    /// # Errors
    /// [`DestError`] saying which step failed.
    fn test(&self) -> Result<(), DestError> {
        let name = format!(".builtpanel-test-{}", test_token());
        let sample = b"builtpanel destination test";
        self.put(&name, &mut &sample[..])?;
        let mut back = Vec::new();
        self.get(&name)?
            .read_to_end(&mut back)
            .map_err(|e| DestError::Other(format!("cannot read back: {}", e.kind())))?;
        self.delete(&name)?;
        if back == sample {
            Ok(())
        } else {
            Err(DestError::Other(
                "what was read back is not what was written".into(),
            ))
        }
    }
}

fn test_token() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{now:x}")
}

/// Whether `name` is an object name: parts separated by `/`, each made of letters, digits
/// and `. _ -`, none starting with `..` or empty; at most 200 characters.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

pub(crate) fn check(name: &str) -> Result<(), DestError> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(DestError::BadName)
    }
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn object_names_are_plain() {
        for good in [
            "a",
            "0190-abc/files.tar.zst.enc",
            ".builtpanel-test-1",
            "a/b/c.json",
        ] {
            assert!(valid_name(good), "{good}");
        }
        for bad in [
            "",
            "/a",
            "a/",
            "a//b",
            "../a",
            "a/../b",
            ".",
            "a/./b",
            "a b",
            "a\0b",
            "é",
            &"x".repeat(201),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
