//! A folder on the machine (the node), reached through `builtpanel-safe-fs`, so no name and
//! no link inside it can lead out of the folder.

use std::{
    io::{Read, Write},
    path::Path,
};

use builtpanel_safe_fs::{FsError, Root};
use rustix::fs::{Mode, OFlags};

use crate::{DestError, Destination, check};

/// A folder.
pub struct Local {
    root: Root,
}

fn fs(e: FsError) -> DestError {
    match e {
        FsError::NotFound => DestError::NotFound,
        FsError::Denied => DestError::Refused("not allowed".into()),
        other => DestError::Other(other.to_string()),
    }
}

impl Local {
    /// Opens the folder.
    ///
    /// # Errors
    /// [`DestError::Config`] when it is not a usable folder.
    pub fn open(dir: &Path) -> Result<Self, DestError> {
        Root::open(dir)
            .map(|root| Self { root })
            .map_err(|e| DestError::Config(format!("the folder: {e}")))
    }

    fn make_folders(&self, name: &str) -> Result<(), DestError> {
        let mut parts: Vec<&str> = name.split('/').collect();
        parts.pop();
        let mut so_far = String::new();
        for part in parts {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            let (dir, leaf) = self.root.parent(&so_far).map_err(fs)?;
            match rustix::fs::mkdirat(&dir, leaf.as_str(), Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(fs(FsError::from(e))),
            }
        }
        Ok(())
    }
}

impl Destination for Local {
    fn put(&self, name: &str, data: &mut dyn Read) -> Result<u64, DestError> {
        check(name)?;
        self.make_folders(name)?;
        let (dir, leaf) = self.root.parent(name).map_err(fs)?;
        let tmp = format!(".{leaf}.part");
        let fd = rustix::fs::openat(
            &dir,
            tmp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| fs(FsError::from(e)))?;
        let mut file = std::fs::File::from(fd);
        let written = (|| -> std::io::Result<u64> {
            let n = std::io::copy(data, &mut file)?;
            file.flush()?;
            file.sync_all()?;
            Ok(n)
        })();
        match written {
            Ok(n) => {
                // Only a whole object is ever under its real name.
                rustix::fs::renameat(&dir, tmp.as_str(), &dir, leaf.as_str())
                    .map_err(|e| fs(FsError::from(e)))?;
                Ok(n)
            }
            Err(e) => {
                let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
                Err(DestError::Other(format!("cannot write: {}", e.kind())))
            }
        }
    }

    fn get(&self, name: &str) -> Result<Box<dyn Read + Send>, DestError> {
        check(name)?;
        let fd = self
            .root
            .open_at(name, OFlags::RDONLY, Mode::empty())
            .map_err(fs)?;
        Ok(Box::new(std::fs::File::from(fd)))
    }

    fn delete(&self, name: &str) -> Result<(), DestError> {
        check(name)?;
        let (dir, leaf) = match self.root.parent(name) {
            Ok(x) => x,
            Err(FsError::NotFound) => return Ok(()),
            Err(e) => return Err(fs(e)),
        };
        match builtpanel_safe_fs::remove(&dir, &leaf, false) {
            Ok(()) | Err(FsError::NotFound) => Ok(()),
            Err(e) => Err(fs(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, DestError> {
        let mut out = Vec::new();
        walk(&self.root, "", &mut out, 0)?;
        out.retain(|n| {
            n.starts_with(prefix) && !n.rsplit('/').next().is_some_and(|l| l.ends_with(".part"))
        });
        out.sort();
        Ok(out)
    }
}

fn walk(root: &Root, dir: &str, out: &mut Vec<String>, depth: usize) -> Result<(), DestError> {
    if depth > 8 {
        return Ok(());
    }
    let fd = root.open_dir(dir).map_err(fs)?;
    let reader = rustix::fs::Dir::read_from(&fd).map_err(|e| fs(FsError::from(e)))?;
    for item in reader {
        let item = item.map_err(|e| fs(FsError::from(e)))?;
        let bytes = item.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let Ok(name) = std::str::from_utf8(bytes) else {
            continue;
        };
        let path = if dir.is_empty() {
            name.to_owned()
        } else {
            format!("{dir}/{name}")
        };
        match item.file_type() {
            rustix::fs::FileType::Directory => walk(root, &path, out, depth + 1)?,
            rustix::fs::FileType::RegularFile => out.push(path),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_stores_lists_reads_and_removes() {
        let dir = std::env::temp_dir().join(format!("bp-dest-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = Local::open(&dir).unwrap();
        assert_eq!(
            d.put("b1/files.tar.zst.enc", &mut &b"hello"[..]).unwrap(),
            5
        );
        d.put("b1/manifest.json.enc", &mut &b"m"[..]).unwrap();
        d.put("b2/x.enc", &mut &b"x"[..]).unwrap();
        assert_eq!(
            d.list("b1/").unwrap(),
            ["b1/files.tar.zst.enc", "b1/manifest.json.enc"]
        );
        let mut got = String::new();
        d.get("b1/files.tar.zst.enc")
            .unwrap()
            .read_to_string(&mut got)
            .unwrap();
        assert_eq!(got, "hello");
        // Replacing is whole; no half object is listed.
        d.put("b1/files.tar.zst.enc", &mut &b"new"[..]).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("b1/files.tar.zst.enc")).unwrap(),
            "new"
        );
        assert!(!d.list("").unwrap().iter().any(|n| n.ends_with(".part")));
        d.delete("b1/files.tar.zst.enc").unwrap();
        d.delete("b1/files.tar.zst.enc").unwrap();
        assert!(matches!(
            d.get("b1/files.tar.zst.enc"),
            Err(DestError::NotFound)
        ));
        // Names and links cannot lead out.
        assert_eq!(d.put("../x", &mut &b"x"[..]), Err(DestError::BadName));
        std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
        assert!(d.put("link/passwd", &mut &b"x"[..]).is_err());
        assert!(d.get("link/passwd").is_err());
        d.test().unwrap();
        assert!(Local::open(&dir.join("nope")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
