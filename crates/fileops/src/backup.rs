//! The files of an account as one encrypted stream (T09.04): a tar, compressed with zstd,
//! cut into authenticated chunks (`builtpanel-backup`).
//!
//! The walk is streaming (nothing is listed first), opens every file from its folder's
//! descriptor without following links, skips links and special files and the trash, and
//! lets the site run meanwhile: a file that changes while it is read gives what it had
//! when it was opened (a file that shrank is padded with zeros, one that grew is cut), so
//! the archive is always whole.

use std::{
    io::{Read, Write},
    os::fd::OwnedFd,
};

use builtpanel_backup::{KEY_LEN, StreamReader, StreamWriter};
use builtpanel_node_protocol::files::{FileErrorCode, TRASH_DIR};
use builtpanel_safe_fs::Root;
use rustix::fs::{FileType, Mode, OFlags};

use super::{
    EntryKind, Out,
    archive::{entry_parts, make_dir, wanted_mode},
    fail, fs, kind_of, names, random_hex, stat_at, sys,
};

const MAX_DEPTH: usize = 128;

/// What was packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Packed {
    /// Files and folders in the archive.
    pub entries: u64,
    /// Bytes of file contents (before compression).
    pub bytes: u64,
    /// Links and special files left out.
    pub skipped: u64,
}

/// Reads the real bytes of a file, then zeros, up to `size`: a file that shrank while it was
/// being packed still gives the length its header promised.
struct Fixed<R: Read> {
    inner: R,
    left: u64,
    ended: bool,
}

impl<R: Read> Read for Fixed<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let want = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = if self.ended {
            0
        } else {
            self.inner.read(&mut buf[..want])?
        };
        if n == 0 {
            self.ended = true;
            buf[..want].fill(0);
            self.left -= want as u64;
            return Ok(want);
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// Packs everything below `root` (the trash excluded) into `out`, encrypted with `key`. With
/// `since` (Unix seconds) only the files that were made or changed after that time are packed
/// (by the later of their modification and status change time, which a copy that keeps the
/// old modification time does not hide); the folders are all there, so the new files have
/// their places.
///
/// # Errors
/// The disk or `out` failing (not a changing file: see the module documentation).
pub fn pack_files(
    root: &Root,
    key: &[u8; KEY_LEN],
    since: Option<i64>,
    out: impl Write,
) -> Out<Packed> {
    let io = |e: std::io::Error| (FileErrorCode::Failed, format!("cannot pack: {}", e.kind()));
    let encrypted = StreamWriter::new(out, key).map_err(io)?;
    let compressed = zstd::stream::Encoder::new(encrypted, 3).map_err(io)?;
    let mut tar = tar::Builder::new(compressed);
    tar.mode(tar::HeaderMode::Deterministic);
    let top = sys(rustix::fs::openat(
        root.fd(),
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    let mut packed = Packed::default();
    walk(&top, "", 0, since, &mut tar, &mut packed)?;
    let compressed = tar.into_inner().map_err(io)?;
    let encrypted = compressed.finish().map_err(io)?;
    encrypted.finish().map_err(io)?;
    Ok(packed)
}

fn walk<W: Write>(
    dir: &OwnedFd,
    path: &str,
    depth: usize,
    since: Option<i64>,
    tar: &mut tar::Builder<W>,
    packed: &mut Packed,
) -> Out<()> {
    let io = |e: std::io::Error| (FileErrorCode::Failed, format!("cannot pack: {}", e.kind()));
    if depth > MAX_DEPTH {
        return fail(FileErrorCode::TooMany, "the files are nested too deep");
    }
    for name in names(dir)? {
        if depth == 0 && name == TRASH_DIR {
            continue;
        }
        // Gone since it was listed: not there any more, nothing to keep.
        let Ok(st) = stat_at(dir, &name) else {
            continue;
        };
        let child = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        let mut header = tar::Header::new_gnu();
        header.set_mode(st.st_mode & 0o777);
        header.set_mtime(u64::try_from(st.st_mtime).unwrap_or(0));
        match kind_of(FileType::from_raw_mode(st.st_mode)) {
            EntryKind::Dir => {
                let fd = match sys(rustix::fs::openat(
                    dir,
                    name.as_str(),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )) {
                    Ok(fd) => fd,
                    Err(_) => continue,
                };
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                tar.append_data(&mut header, format!("{child}/"), std::io::empty())
                    .map_err(io)?;
                packed.entries += 1;
                walk(&fd, &child, depth + 1, since, tar, packed)?;
            }
            EntryKind::File => {
                if since.is_some_and(|t| st.st_mtime.max(st.st_ctime) <= t) {
                    continue;
                }
                let Ok(fd) = sys(rustix::fs::openat(
                    dir,
                    name.as_str(),
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )) else {
                    continue;
                };
                // The size of the file that is open, not of the one that was listed.
                let size = sys(rustix::fs::fstat(&fd))
                    .map(|s| u64::try_from(s.st_size).unwrap_or(0))
                    .unwrap_or(0);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(size);
                let reader = Fixed {
                    inner: std::fs::File::from(fd),
                    left: size,
                    ended: false,
                };
                tar.append_data(&mut header, &child, reader).map_err(io)?;
                packed.entries += 1;
                packed.bytes += size;
            }
            EntryKind::Link | EntryKind::Other => packed.skipped += 1,
        }
    }
    Ok(())
}

/// What was unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unpacked {
    /// Files and folders written.
    pub entries: u64,
    /// Bytes of file contents written.
    pub bytes: u64,
    /// Entries left out (links, special files, the trash, a folder where a file is).
    pub skipped: u64,
}

/// Puts the files of a stream made by [`pack_files`] back below `root`, over what is there
/// (files that are not in the stream stay). With `only`, just that folder (and what is
/// below it) is put back. Every name is checked against escaping the folder, every file is
/// written beside its place and moved in at the end, so a cut stream leaves no half file.
///
/// # Errors
/// A wrong key, a changed or cut stream (the entries before it are already back), a bad
/// name, or the disk failing.
pub fn unpack_files(
    root: &Root,
    key: &[u8; KEY_LEN],
    input: impl Read,
    only: Option<&str>,
) -> Out<Unpacked> {
    let io = |e: std::io::Error| {
        (
            FileErrorCode::Failed,
            format!("cannot unpack: {}", e.kind()),
        )
    };
    let only: Option<Vec<String>> = only
        .filter(|f| !f.is_empty() && *f != "/")
        .map(entry_parts)
        .transpose()?;
    let decrypted = StreamReader::new(input, key);
    let plain = zstd::stream::Decoder::new(decrypted).map_err(io)?;
    let mut archive = tar::Archive::new(plain);
    let mut done = Unpacked::default();
    for entry in archive.entries().map_err(io)? {
        let mut entry = entry.map_err(io)?;
        let raw = entry
            .path()
            .map_err(|_| {
                (
                    FileErrorCode::InvalidPath,
                    "an entry has no usable name".to_owned(),
                )
            })?
            .to_string_lossy()
            .into_owned();
        let parts = entry_parts(raw.trim_end_matches('/'))?;
        let wanted = only
            .as_ref()
            .is_none_or(|o| parts.len() >= o.len() && parts[..o.len()] == o[..]);
        if !wanted || parts.first().is_some_and(|p| p == TRASH_DIR) {
            done.skipped += 1;
            continue;
        }
        let mode = entry.header().mode().unwrap_or(0o644);
        let mtime = entry.header().mtime().unwrap_or(0);
        let (last, folders) = parts.split_last().ok_or_else(|| {
            (
                FileErrorCode::InvalidPath,
                "an entry names nothing".to_owned(),
            )
        })?;
        let mut rel = String::new();
        for folder in folders {
            if !rel.is_empty() {
                rel.push('/');
            }
            rel.push_str(folder);
            make_dir(root, &rel, 0o755)?;
        }
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push_str(last);
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                make_dir(root, &rel, wanted_mode(mode, true))?;
                done.entries += 1;
            }
            tar::EntryType::Regular => {
                let (dir, name) = fs(root.parent(&rel))?;
                // A folder where the backup has a file stays: it is not ours to remove.
                if let Ok(st) = stat_at(&dir, &name)
                    && kind_of(FileType::from_raw_mode(st.st_mode)) == EntryKind::Dir
                {
                    done.skipped += 1;
                    continue;
                }
                let tmp = format!(".bp-rs-{}", random_hex(8)?);
                let fd = sys(rustix::fs::openat(
                    &dir,
                    tmp.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(0o600),
                ))?;
                let size = entry.header().size().unwrap_or(0);
                let mut file = std::fs::File::from(fd);
                let copied = std::io::copy(&mut (&mut entry).take(size), &mut file);
                let ok = copied.as_ref().is_ok_and(|n| *n == size);
                if !ok {
                    let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
                    return Err(match copied {
                        Err(e) => io(e),
                        Ok(_) => (
                            FileErrorCode::Failed,
                            "an entry is shorter than it says".to_owned(),
                        ),
                    });
                }
                let _ = rustix::fs::fchmod(&file, Mode::from_raw_mode(wanted_mode(mode, false)));
                let _ = rustix::fs::futimens(
                    &file,
                    &rustix::fs::Timestamps {
                        last_access: rustix::fs::Timespec {
                            tv_sec: i64::try_from(mtime).unwrap_or(0),
                            tv_nsec: 0,
                        },
                        last_modification: rustix::fs::Timespec {
                            tv_sec: i64::try_from(mtime).unwrap_or(0),
                            tv_nsec: 0,
                        },
                    },
                );
                if let Err(e) = rustix::fs::renameat(&dir, tmp.as_str(), &dir, name.as_str()) {
                    let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
                    return Err(super::from_fs(builtpanel_safe_fs::FsError::from(e)));
                }
                done.entries += 1;
                done.bytes += size;
            }
            _ => done.skipped += 1,
        }
    }
    Ok(done)
}
