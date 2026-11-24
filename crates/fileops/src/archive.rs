//! Packing and unpacking (T08.05).
//!
//! Unpacking is two passes over the archive: the first reads only names, kinds and
//! sizes and refuses the whole archive for an entry that would leave the folder
//! (`../../etc/passwd`, an absolute name), for too many entries or too many bytes;
//! the second writes, through `builtpanel-safe-fs`, so even a folder that turns into a
//! link between the passes cannot be followed. Links, devices and the like inside an
//! archive are skipped, never created; the permission bits that come with an entry
//! are cut to `0777` (no setuid) and always leave the owner able to use the file.

use std::{
    io::{Read, Seek, Write as _},
    os::fd::OwnedFd,
};

use builtpanel_node_protocol::files::{ArchiveFormat, FileErrorCode, FileResult};
use builtpanel_safe_fs::{FsError, Root};
use rustix::fs::{Mode, OFlags};

use super::{EntryKind, Out, fail, fs, kind_of, names, stat_at, sys, user_path};

const MAX_DEPTH: usize = 64;

/// The first bytes of a zip file and of a gzip file.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];

fn bad(message: &str) -> (FileErrorCode, String) {
    (FileErrorCode::BadArchive, message.to_owned())
}

// -------------------------------------------------------------------- pack

struct Item {
    /// Name inside the archive.
    name: String,
    dir: bool,
    mode: u32,
    mtime: i64,
    /// Where to read it from (files only).
    from: Option<(OwnedFd, String)>,
    size: u64,
}

fn collect(
    dir: &OwnedFd,
    name: &str,
    inside: &str,
    depth: usize,
    out: &mut Vec<Item>,
    total: &mut u64,
    max_bytes: u64,
) -> Out<()> {
    if depth > MAX_DEPTH || out.len() > 200_000 {
        return fail(FileErrorCode::TooMany, "too many or too deep");
    }
    let st = stat_at(dir, name)?;
    let size = u64::try_from(st.st_size).unwrap_or(0);
    match kind_of(rustix::fs::FileType::from_raw_mode(st.st_mode)) {
        EntryKind::File => {
            *total += size;
            if *total > max_bytes {
                return fail(
                    FileErrorCode::TooLarge,
                    "the files add up to more than the limit",
                );
            }
            out.push(Item {
                name: inside.to_owned(),
                dir: false,
                mode: st.st_mode & 0o777,
                mtime: st.st_mtime,
                from: Some((sys(rustix::io::dup(dir))?, name.to_owned())),
                size,
            });
        }
        EntryKind::Dir => {
            out.push(Item {
                name: format!("{inside}/"),
                dir: true,
                mode: st.st_mode & 0o777,
                mtime: st.st_mtime,
                from: None,
                size: 0,
            });
            let fd = sys(rustix::fs::openat(
                dir,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
            for child in names(&fd)? {
                collect(
                    &fd,
                    &child,
                    &format!("{inside}/{child}"),
                    depth + 1,
                    out,
                    total,
                    max_bytes,
                )?;
            }
        }
        // Links and special files are not packed.
        EntryKind::Link | EntryKind::Other => {}
    }
    Ok(())
}

pub(super) fn archive(
    root: &Root,
    paths: &[String],
    dest: &str,
    format: ArchiveFormat,
    max_bytes: u64,
) -> Out<FileResult> {
    if paths.is_empty() || paths.len() > 1000 {
        return fail(FileErrorCode::InvalidArguments, "pack 1 to 1000 things");
    }
    let (dest_dir, dest_name) = fs(root.parent(user_path(dest)?))?;
    let mut items = Vec::new();
    let mut total = 0;
    for path in paths {
        let (dir, name) = fs(root.parent(user_path(path)?))?;
        collect(
            &dir,
            &name,
            &name.clone(),
            0,
            &mut items,
            &mut total,
            max_bytes,
        )?;
    }
    let file = sys(rustix::fs::openat(
        &dest_dir,
        dest_name.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o644),
    ))?;
    let written = write_archive(std::fs::File::from(file), &items, format);
    match written {
        Ok(()) => Ok(FileResult::Done {
            count: u32::try_from(items.len()).unwrap_or(u32::MAX),
        }),
        Err(message) => {
            let _ = builtpanel_safe_fs::remove(&dest_dir, &dest_name, false);
            Err((FileErrorCode::Failed, message))
        }
    }
}

fn write_archive(file: std::fs::File, items: &[Item], format: ArchiveFormat) -> Result<(), String> {
    let io = |e: std::io::Error| format!("cannot pack: {}", e.kind());
    let open = |item: &Item| -> Result<std::fs::File, String> {
        let (dir, name) = item.from.as_ref().ok_or("not a file")?;
        let fd = rustix::fs::openat(
            dir,
            name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| format!("cannot read: {e}"))?;
        Ok(std::fs::File::from(fd))
    };
    match format {
        ArchiveFormat::Zip => {
            let mut zip = zip::ZipWriter::new(file);
            for item in items {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(item.mode)
                    .large_file(item.size > u64::from(u32::MAX) / 2);
                if item.dir {
                    zip.add_directory(item.name.as_str(), options)
                        .map_err(|e| format!("cannot pack: {e}"))?;
                } else {
                    zip.start_file(item.name.as_str(), options)
                        .map_err(|e| format!("cannot pack: {e}"))?;
                    // At most the size counted: a file that grew meanwhile is cut.
                    std::io::copy(&mut open(item)?.take(item.size), &mut zip).map_err(io)?;
                }
            }
            zip.finish()
                .map_err(|e| format!("cannot pack: {e}"))?
                .flush()
                .map_err(io)
        }
        ArchiveFormat::TarGz => {
            let gz = flate2::write::GzEncoder::new(file, flate2::Compression::new(6));
            let mut tar = tar::Builder::new(gz);
            tar.mode(tar::HeaderMode::Deterministic);
            for item in items {
                let mut header = tar::Header::new_gnu();
                header.set_mode(item.mode);
                header.set_mtime(u64::try_from(item.mtime).unwrap_or(0));
                header.set_uid(0);
                header.set_gid(0);
                if item.dir {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    tar.append_data(
                        &mut header,
                        item.name.trim_end_matches('/'),
                        std::io::empty(),
                    )
                    .map_err(io)?;
                } else {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(item.size);
                    tar.append_data(&mut header, item.name.as_str(), open(item)?.take(item.size))
                        .map_err(io)?;
                }
            }
            tar.into_inner()
                .and_then(flate2::write::GzEncoder::finish)
                .map_err(io)?
                .flush()
                .map_err(io)
        }
    }
}

// ------------------------------------------------------------------ unpack

/// One entry as the first pass sees it.
struct Seen {
    parts: Vec<String>,
    dir: bool,
    size: u64,
    mode: u32,
    skip: bool,
}

/// Checks a name from an archive and splits it.
pub(super) fn entry_parts(raw: &str) -> Out<Vec<String>> {
    let parts =
        builtpanel_safe_fs::parts(raw).map_err(|_| bad("an entry would leave the folder"))?;
    // A name with a backslash is a Windows path in disguise.
    if parts.iter().any(|p| p.contains('\\')) {
        return Err(bad("an entry has a backslash in its name"));
    }
    Ok(parts.into_iter().map(str::to_owned).collect())
}

pub(super) fn wanted_mode(mode: u32, dir: bool) -> u32 {
    let mode = mode & 0o777;
    if dir { mode | 0o700 } else { mode | 0o600 }
}

pub(super) fn extract(
    root: &Root,
    path: &str,
    dest: &str,
    max_bytes: u64,
    max_entries: u32,
) -> Out<FileResult> {
    let dest_parts = fs(builtpanel_safe_fs::parts(user_path(dest)?).map_err(FsError::Path))?;
    let archive_fd = fs(root.open_at(
        user_path(path)?,
        OFlags::RDONLY | OFlags::NONBLOCK,
        Mode::empty(),
    ))?;
    let st = sys(rustix::fs::fstat(&archive_fd))?;
    if kind_of(rustix::fs::FileType::from_raw_mode(st.st_mode)) != EntryKind::File {
        return fail(FileErrorCode::InvalidArguments, "not a file");
    }
    let mut file = std::fs::File::from(archive_fd);
    let mut magic = [0u8; 4];
    let n = file
        .read(&mut magic)
        .or_else(|_| fail(FileErrorCode::Failed, "cannot read"))?;
    file.rewind()
        .or_else(|_| fail(FileErrorCode::Failed, "cannot read"))?;
    let zip = magic[..n].starts_with(ZIP_MAGIC);
    if !zip && !magic[..n].starts_with(GZIP_MAGIC) {
        return Err(bad("not a zip or tar.gz file"));
    }
    // Pass 1: everything is looked at, nothing is written.
    let mut seen: Vec<Seen> = Vec::new();
    if zip {
        scan_zip(&mut file, &mut seen, max_bytes, max_entries)?;
    } else {
        scan_tar(&mut file, &mut seen, max_bytes, max_entries)?;
    }
    file.rewind()
        .or_else(|_| fail(FileErrorCode::Failed, "cannot read"))?;
    // The folder to unpack into.
    let mut so_far = String::new();
    for part in &dest_parts {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        make_dir(root, &so_far, 0o755)?;
    }
    // Pass 2.
    let mut count = 0;
    let base = dest_parts.join("/");
    if zip {
        let mut archive = zip::ZipArchive::new(file).map_err(|_| bad("damaged zip file"))?;
        for (i, item) in seen.iter().enumerate() {
            if item.skip {
                continue;
            }
            let mut entry = archive.by_index(i).map_err(|_| bad("damaged zip file"))?;
            put(root, &base, item, &mut entry, max_bytes)?;
            count += 1;
        }
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let entries = archive.entries().map_err(|_| bad("damaged tar.gz file"))?;
        for (entry, item) in entries.zip(seen.iter()) {
            let mut entry = entry.map_err(|_| bad("damaged tar.gz file"))?;
            if item.skip {
                continue;
            }
            put(root, &base, item, &mut entry, max_bytes)?;
            count += 1;
        }
    }
    Ok(FileResult::Done { count })
}

fn scan_zip(
    file: &mut std::fs::File,
    seen: &mut Vec<Seen>,
    max_bytes: u64,
    max_entries: u32,
) -> Out<()> {
    let mut archive = zip::ZipArchive::new(file).map_err(|_| bad("damaged zip file"))?;
    if archive.len() > max_entries as usize {
        return fail(FileErrorCode::TooMany, "the archive has too many entries");
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|_| bad("damaged zip file"))?;
        let name = entry.name().map_err(|_| bad("an entry has a bad name"))?;
        let parts = entry_parts(&name)?;
        let link = entry
            .unix_mode()
            .is_some_and(|m| m & 0o170_000 == 0o120_000);
        let special = entry
            .unix_mode()
            .is_some_and(|m| !matches!(m & 0o170_000, 0 | 0o100_000 | 0o040_000));
        total = total.saturating_add(entry.size());
        if total > max_bytes {
            return fail(
                FileErrorCode::TooLarge,
                "the archive unpacks to more than the limit",
            );
        }
        seen.push(Seen {
            skip: parts.is_empty() || link || special,
            dir: entry.is_dir(),
            size: entry.size(),
            mode: entry
                .unix_mode()
                .unwrap_or(if entry.is_dir() { 0o755 } else { 0o644 }),
            parts,
        });
    }
    Ok(())
}

fn scan_tar(
    file: &mut std::fs::File,
    seen: &mut Vec<Seen>,
    max_bytes: u64,
    max_entries: u32,
) -> Out<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut total = 0u64;
    for entry in archive.entries().map_err(|_| bad("damaged tar.gz file"))? {
        let entry = entry.map_err(|_| bad("damaged tar.gz file"))?;
        if seen.len() >= max_entries as usize {
            return fail(FileErrorCode::TooMany, "the archive has too many entries");
        }
        let raw = entry.path().map_err(|_| bad("an entry has a bad name"))?;
        let raw = raw
            .to_str()
            .ok_or_else(|| bad("an entry has a bad name"))?
            .to_owned();
        let parts = entry_parts(&raw)?;
        let kind = entry.header().entry_type();
        let dir = kind.is_dir();
        let regular = matches!(kind, tar::EntryType::Regular | tar::EntryType::Continuous);
        total = total.saturating_add(entry.size());
        if total > max_bytes {
            return fail(
                FileErrorCode::TooLarge,
                "the archive unpacks to more than the limit",
            );
        }
        seen.push(Seen {
            skip: parts.is_empty() || !(dir || regular),
            dir,
            size: entry.size(),
            mode: entry.header().mode().unwrap_or(0o644),
            parts,
        });
    }
    Ok(())
}

pub(super) fn make_dir(root: &Root, rel: &str, mode: u32) -> Out<()> {
    let (parent, name) = fs(root.parent(rel))?;
    match rustix::fs::mkdirat(&parent, name.as_str(), Mode::from_raw_mode(mode)) {
        Ok(()) => Ok(()),
        // Already there: fine for a folder, and the next step opens it the safe way.
        Err(rustix::io::Errno::EXIST) => fs(root.open_dir(rel)).map(|_| ()),
        Err(e) => Err(super::from_fs(FsError::from(e))),
    }
}

/// Writes one entry below `base`, making the folders above it.
fn put(root: &Root, base: &str, item: &Seen, reader: &mut dyn Read, max_bytes: u64) -> Out<()> {
    let mut rel = base.to_owned();
    let (last, folders) = item
        .parts
        .split_last()
        .ok_or_else(|| bad("an entry names nothing"))?;
    let join = |rel: &mut String, part: &str| {
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push_str(part);
    };
    for folder in folders {
        join(&mut rel, folder);
        make_dir(root, &rel, 0o755)?;
    }
    join(&mut rel, last);
    if item.dir {
        make_dir(root, &rel, wanted_mode(item.mode, true))?;
        return Ok(());
    }
    let (parent, name) = fs(root.parent(&rel))?;
    let fd = sys(rustix::fs::openat(
        &parent,
        name.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(wanted_mode(item.mode, false)),
    ))?;
    let mut out = std::fs::File::from(fd);
    // What the archive says it holds is a promise the data is held to.
    let promised = item.size.min(max_bytes);
    let copied = std::io::copy(&mut reader.take(promised + 1), &mut out);
    let ok = copied.is_ok_and(|n| n == item.size);
    if !ok {
        let _ = builtpanel_safe_fs::remove(&parent, &name, false);
        return Err(bad("an entry is not the size it says"));
    }
    // The mode of a new file is cut by the umask; set exactly what was asked.
    let _ = rustix::fs::fchmod(&out, Mode::from_raw_mode(wanted_mode(item.mode, false)));
    Ok(())
}
