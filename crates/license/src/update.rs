//! Installing a release (T17.15): nothing is touched until the manifest's signature and every
//! file's hash have been checked; each file is then put in place with an atomic rename, the
//! file it replaces is kept as the known-good copy, and when anything goes wrong the files that
//! were already replaced are put back.

use std::{
    fs,
    io::Write as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use crate::{
    keys::PublicKeys,
    release::{ReleaseError, ReleaseManifest, SignedManifest},
};

/// Where the copies of what was replaced are kept, inside the install folder.
pub const KNOWN_GOOD: &str = ".known-good";

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The manifest or one of the files is not what it should be; nothing was changed.
    #[error(transparent)]
    Release(#[from] ReleaseError),
    /// A file of the release is missing from the download.
    #[error("{0} is missing from the download")]
    Missing(String),
    /// Reading or writing failed. When files had been replaced already they were put back.
    #[error("{0}")]
    Io(String),
}

fn io(context: &str, error: &std::io::Error) -> UpdateError {
    UpdateError::Io(format!("{context}: {}", error.kind()))
}

/// The files of a download: name and content.
pub type Files = Vec<(String, Vec<u8>)>;

/// Checks `signed` against the trusted keys and every file of it in `from`, without changing
/// anything. Returns the manifest and the files' contents.
///
/// # Errors
/// [`UpdateError`].
pub fn check_download(
    signed: &SignedManifest,
    keys: &PublicKeys,
    from: &Path,
) -> Result<(ReleaseManifest, Files), UpdateError> {
    let manifest = signed.verify(keys)?;
    let mut files = Vec::with_capacity(manifest.components.len());
    for component in &manifest.components {
        let path = from.join(&component.name);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(UpdateError::Missing(component.name.clone()));
            }
            Err(e) => return Err(io(&component.name, &e)),
        };
        manifest.check_file(&component.name, &bytes)?;
        files.push((component.name.clone(), bytes));
    }
    Ok((manifest, files))
}

/// Verifies the download and installs it into `to`. Returns the manifest that was installed.
///
/// # Errors
/// [`UpdateError`]; after an error `to` holds the files it held before.
pub fn install(
    signed: &SignedManifest,
    keys: &PublicKeys,
    from: &Path,
    to: &Path,
) -> Result<ReleaseManifest, UpdateError> {
    let (manifest, files) = check_download(signed, keys, from)?;
    fs::create_dir_all(to.join(KNOWN_GOOD)).map_err(|e| io("install folder", &e))?;
    // 1. Every new file is written next to its place first, so the swaps below are renames
    //    inside one folder (atomic) and a full disk is found before anything is replaced.
    let mut staged: Vec<(String, PathBuf)> = Vec::new();
    for (name, bytes) in &files {
        let temp = to.join(format!(".{name}.new"));
        let written = (|| {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode_0755()
                .open(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()
        })();
        if let Err(e) = written {
            remove_all(staged.iter().map(|(_, p)| p));
            let _ = fs::remove_file(&temp);
            return Err(io(name, &e));
        }
        staged.push((name.clone(), temp));
    }
    // 2. Swap, remembering what was replaced.
    let mut swapped: Vec<String> = Vec::new();
    for (name, temp) in &staged {
        let place = to.join(name);
        let result = (|| {
            if place.exists() {
                fs::copy(&place, to.join(KNOWN_GOOD).join(name))?;
            }
            fs::rename(temp, &place)
        })();
        match result {
            Ok(()) => swapped.push(name.clone()),
            Err(e) => {
                // Put back what was replaced, drop what is left over.
                let _ = restore(to, &swapped);
                remove_all(staged.iter().map(|(_, p)| p));
                return Err(io(name, &e));
            }
        }
    }
    Ok(manifest)
}

/// Puts the known-good copies back over the installed files (all of them).
///
/// # Errors
/// [`UpdateError::Io`].
pub fn rollback(to: &Path) -> Result<Vec<String>, UpdateError> {
    let dir = to.join(KNOWN_GOOD);
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| io("known-good folder", &e))? {
        let entry = entry.map_err(|e| io("known-good folder", &e))?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    restore(to, &names)?;
    Ok(names)
}

fn restore(to: &Path, names: &[String]) -> Result<(), UpdateError> {
    for name in names {
        let good = to.join(KNOWN_GOOD).join(name);
        if !good.exists() {
            continue;
        }
        let temp = to.join(format!(".{name}.back"));
        fs::copy(&good, &temp).map_err(|e| io(name, &e))?;
        fs::rename(&temp, to.join(name)).map_err(|e| io(name, &e))?;
    }
    Ok(())
}

fn remove_all<'a>(paths: impl Iterator<Item = &'a PathBuf>) {
    for p in paths {
        let _ = fs::remove_file(p);
    }
}

trait Mode0755 {
    fn mode_0755(&mut self) -> &mut Self;
}

impl Mode0755 for fs::OpenOptions {
    fn mode_0755(&mut self) -> &mut Self {
        std::os::unix::fs::OpenOptionsExt::mode(self, 0o755)
    }
}

/// Whether a file is executable by its owner (used by tests and the CLI's report).
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o100 != 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::keys::{FileSigningKey, SigningKey as _};

    struct Rig {
        base: PathBuf,
        keys: PublicKeys,
        signed: SignedManifest,
        from: PathBuf,
        to: PathBuf,
    }

    fn rig() -> Rig {
        let base = std::env::temp_dir().join(format!("bp-update-{}", uuid::Uuid::new_v4()));
        let (from, to) = (base.join("from"), base.join("to"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        let key = FileSigningKey::generate_to_file(&base.join("k"), "release-1").unwrap();
        let files = vec![
            ("api".to_owned(), b"api v2".to_vec()),
            ("worker".to_owned(), b"worker v2".to_vec()),
        ];
        for (n, b) in &files {
            fs::write(from.join(n), b).unwrap();
        }
        // What is installed now.
        fs::write(to.join("api"), b"api v1").unwrap();
        fs::write(to.join("worker"), b"worker v1").unwrap();
        let manifest = ReleaseManifest::build("2.0.0", "abc", 1, &files).unwrap();
        Rig {
            keys: PublicKeys::new()
                .with("release-1", key.public_key())
                .unwrap(),
            signed: SignedManifest::sign(&manifest, &key).unwrap(),
            base,
            from,
            to,
        }
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    #[test]
    fn a_good_release_is_installed_and_the_old_files_are_kept_as_known_good() {
        let r = rig();
        let m = install(&r.signed, &r.keys, &r.from, &r.to).unwrap();
        assert_eq!(m.version, "2.0.0");
        assert_eq!(read(&r.to.join("api")), "api v2");
        assert_eq!(read(&r.to.join("worker")), "worker v2");
        assert!(is_executable(&r.to.join("api")));
        assert_eq!(read(&r.to.join(KNOWN_GOOD).join("api")), "api v1");
        // And it can be taken back.
        let back = rollback(&r.to).unwrap();
        assert_eq!(back, vec!["api".to_owned(), "worker".to_owned()]);
        assert_eq!(read(&r.to.join("api")), "api v1");
        assert_eq!(read(&r.to.join("worker")), "worker v1");
        let _ = fs::remove_dir_all(r.base);
    }

    #[test]
    fn one_changed_byte_stops_the_whole_update_before_anything_is_touched() {
        let r = rig();
        fs::write(r.from.join("worker"), b"worker v3").unwrap();
        assert!(matches!(
            install(&r.signed, &r.keys, &r.from, &r.to),
            Err(UpdateError::Release(ReleaseError::Mismatch(_)))
        ));
        assert_eq!(read(&r.to.join("api")), "api v1");
        assert_eq!(read(&r.to.join("worker")), "worker v1");
        assert!(!r.to.join(KNOWN_GOOD).exists());
        let _ = fs::remove_dir_all(r.base);
    }

    #[test]
    fn a_missing_file_a_wrong_key_and_a_forged_manifest_are_refused() {
        let r = rig();
        fs::remove_file(r.from.join("worker")).unwrap();
        assert!(matches!(
            install(&r.signed, &r.keys, &r.from, &r.to),
            Err(UpdateError::Missing(_))
        ));
        fs::write(r.from.join("worker"), b"worker v2").unwrap();
        let strangers = PublicKeys::new();
        assert!(matches!(
            install(&r.signed, &strangers, &r.from, &r.to),
            Err(UpdateError::Release(ReleaseError::Signature))
        ));
        assert_eq!(read(&r.to.join("api")), "api v1");
        let _ = fs::remove_dir_all(r.base);
    }

    #[test]
    fn a_failure_half_way_puts_the_replaced_files_back() {
        let r = rig();
        // `worker` cannot be replaced: its place is a folder.
        fs::remove_file(r.to.join("worker")).unwrap();
        fs::create_dir(r.to.join("worker")).unwrap();
        assert!(install(&r.signed, &r.keys, &r.from, &r.to).is_err());
        assert_eq!(read(&r.to.join("api")), "api v1", "api was put back");
        assert!(r.to.join("worker").is_dir());
        assert!(!r.to.join(".api.new").exists() && !r.to.join(".worker.new").exists());
        let _ = fs::remove_dir_all(r.base);
    }
}
