//! The file manager's operations on one tenant's files (T08.02-T08.06).
//!
//! [`handle`] takes a request and a [`Root`] (the tenant's directory) and does it,
//! as whoever the process is: the privileged helper runs it as the tenant's own
//! user id, in a process of its own, so what a tenant can do here is exactly what
//! the tenant could do with the files anyway. Every path goes through
//! `builtpanel-safe-fs`, which follows no links and cannot leave the root.
//!
//! Nothing here replaces a file silently except [`FileRequest::Write`] (atomic:
//! a temporary file in the same directory, then a rename) and nothing removes a
//! file for good except emptying the trash.

mod archive;
pub mod backup;
mod usage;

use std::{
    io::{Read as _, Write as _},
    os::fd::{AsFd, OwnedFd},
};

use builtpanel_node_protocol::files::{
    EntryKind, FileEntry, FileErrorCode, FileRequest, FileResult, MAX_DIR_ENTRIES, MAX_EDIT_BYTES,
    MAX_LIST_LIMIT, TRASH_DIR, TrashItem,
};
use builtpanel_safe_fs::{FsError, PathError, Root};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, Stat, fstat, statat};

/// Most items one copy, fix or empty touches.
const MAX_TREE_ITEMS: u32 = 200_000;
/// Deepest tree walked.
const MAX_TREE_DEPTH: usize = 64;

type Out<T> = Result<T, (FileErrorCode, String)>;

fn fail<T>(code: FileErrorCode, message: &str) -> Out<T> {
    Err((code, message.to_owned()))
}

fn from_fs(e: FsError) -> (FileErrorCode, String) {
    let (code, text) = match e {
        FsError::Path(PathError::Empty) => (FileErrorCode::InvalidPath, "the path names nothing"),
        FsError::Path(_) => (FileErrorCode::InvalidPath, "the path is not acceptable"),
        FsError::Link => (FileErrorCode::Link, "a link is in the way"),
        FsError::NotFound => (FileErrorCode::NotFound, "not found"),
        FsError::Exists => (FileErrorCode::Exists, "already exists"),
        FsError::NotDir => (FileErrorCode::NotDir, "not a directory"),
        FsError::IsDir => (FileErrorCode::IsDir, "is a directory"),
        FsError::Denied => (FileErrorCode::Denied, "not allowed"),
        FsError::Other(_) => (FileErrorCode::Failed, "the system refused"),
    };
    (code, text.to_owned())
}

fn fs<T>(r: Result<T, FsError>) -> Out<T> {
    r.map_err(from_fs)
}

fn sys<T>(r: Result<T, rustix::io::Errno>) -> Out<T> {
    r.map_err(|e| from_fs(FsError::from(e)))
}

/// Does one request.
#[must_use]
pub fn handle(root: &Root, request: FileRequest) -> FileResult {
    match run(root, request) {
        Ok(result) => result,
        Err((code, message)) => FileResult::Error { code, message },
    }
}

fn run(root: &Root, request: FileRequest) -> Out<FileResult> {
    match request {
        FileRequest::List {
            path,
            offset,
            limit,
        } => list(root, &path, offset, limit),
        FileRequest::Stat { path } => stat(root, &path),
        FileRequest::Read { path, max_bytes } => read(root, &path, max_bytes),
        FileRequest::Write {
            path,
            content,
            max_bytes,
            expected_mtime,
            create_only,
        } => write(
            root,
            &path,
            &content,
            max_bytes,
            expected_mtime,
            create_only,
        ),
        FileRequest::Mkdir { path } => mkdir(root, &path),
        FileRequest::Move { from, to } => rename(root, &from, &to),
        FileRequest::Copy { from, to } => copy(root, &from, &to),
        FileRequest::Delete { path } => trash::delete(root, &path),
        FileRequest::TrashList => trash::list(root),
        FileRequest::Restore { id } => trash::restore(root, &id),
        FileRequest::EmptyTrash {
            id,
            older_than_days,
        } => trash::empty(root, id.as_deref(), older_than_days),
        FileRequest::Archive {
            paths,
            dest,
            format,
            max_bytes,
        } => archive::archive(root, &paths, &dest, format, max_bytes),
        FileRequest::Extract {
            path,
            dest,
            max_bytes,
            max_entries,
        } => archive::extract(root, &path, &dest, max_bytes, max_entries),
        FileRequest::DiskUsage { top } => usage::disk_usage(root, top),
        FileRequest::Chmod {
            path,
            mode,
            recursive,
        } => chmod(root, &path, mode, recursive),
        FileRequest::FixPermissions { path } => fix_permissions(root, &path),
    }
}

// ----------------------------------------------------------------- helpers

/// A path of the tenant's own files: the trash folder is not one.
fn user_path(path: &str) -> Out<&str> {
    let parts = fs(builtpanel_safe_fs::parts(path).map_err(FsError::Path))?;
    if parts.first() == Some(&TRASH_DIR) {
        return fail(FileErrorCode::InvalidPath, "the trash folder is not a path");
    }
    Ok(path)
}

fn kind_of(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Dir,
        FileType::Symlink => EntryKind::Link,
        _ => EntryKind::Other,
    }
}

fn entry(name: &str, st: &Stat) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        kind: kind_of(FileType::from_raw_mode(st.st_mode)),
        size: u64::try_from(st.st_size).unwrap_or(0),
        mtime: st.st_mtime,
        mode: st.st_mode & 0o7777,
    }
}

fn stat_at(dir: &impl AsFd, name: &str) -> Out<Stat> {
    sys(statat(dir, name, AtFlags::SYMLINK_NOFOLLOW))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The names in a directory (without `.` and `..`), sorted, at most
/// [`MAX_DIR_ENTRIES`].
fn names(dir: &OwnedFd) -> Out<Vec<String>> {
    let reader = sys(Dir::read_from(dir))?;
    let mut out = Vec::new();
    for item in reader {
        let item = sys(item)?;
        let bytes = item.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        // A name that is not UTF-8 cannot be shown or sent; it is left out.
        if let Ok(name) = std::str::from_utf8(bytes) {
            out.push(name.to_owned());
        }
        if out.len() >= MAX_DIR_ENTRIES {
            break;
        }
    }
    out.sort();
    Ok(out)
}

fn random_hex(bytes: usize) -> Out<String> {
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .or_else(|_| fail(FileErrorCode::Failed, "no randomness"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

// -------------------------------------------------------------------- list

fn list(root: &Root, path: &str, offset: u32, limit: u32) -> Out<FileResult> {
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return fail(
            FileErrorCode::InvalidArguments,
            "the page size is 1 to 1000",
        );
    }
    let at_root = fs(builtpanel_safe_fs::parts(path).map_err(FsError::Path))?.is_empty();
    if !at_root {
        user_path(path)?;
    }
    let dir = fs(root.open_dir(path))?;
    let mut all = names(&dir)?;
    if at_root {
        all.retain(|n| n != TRASH_DIR);
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    for name in &all {
        // Gone since the names were read: skipped.
        if let Ok(st) = stat_at(&dir, name) {
            entries.push(entry(name, &st));
        }
    }
    // Folders first, then by name (the names are sorted already; this is stable).
    entries.sort_by_key(|e| e.kind != EntryKind::Dir);
    let total = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let page = entries
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    Ok(FileResult::Entries {
        entries: page,
        total,
    })
}

fn stat(root: &Root, path: &str) -> Out<FileResult> {
    let (dir, name) = fs(root.parent(user_path(path)?))?;
    let st = stat_at(&dir, &name)?;
    Ok(FileResult::Entry {
        entry: entry(&name, &st),
    })
}

// ------------------------------------------------------------ read / write

fn read(root: &Root, path: &str, max_bytes: u32) -> Out<FileResult> {
    let max = max_bytes.min(MAX_EDIT_BYTES);
    let fd = fs(root.open_at(
        user_path(path)?,
        OFlags::RDONLY | OFlags::NONBLOCK,
        Mode::empty(),
    ))?;
    let st = sys(fstat(&fd))?;
    match kind_of(FileType::from_raw_mode(st.st_mode)) {
        EntryKind::File => {}
        EntryKind::Dir => return fail(FileErrorCode::IsDir, "is a directory"),
        _ => return fail(FileErrorCode::InvalidArguments, "not a regular file"),
    }
    let size = u64::try_from(st.st_size).unwrap_or(0);
    if size > u64::from(max) {
        return fail(FileErrorCode::TooLarge, "the file is bigger than the limit");
    }
    let mut bytes = Vec::new();
    // One more byte than allowed: a file that grew since `fstat` is caught too.
    std::fs::File::from(fd)
        .take(u64::from(max) + 1)
        .read_to_end(&mut bytes)
        .or_else(|_| fail(FileErrorCode::Failed, "cannot read"))?;
    if bytes.len() as u64 > u64::from(max) {
        return fail(FileErrorCode::TooLarge, "the file is bigger than the limit");
    }
    let (content, binary) = match String::from_utf8(bytes) {
        Ok(text) => (Some(text), false),
        Err(_) => (None, true),
    };
    Ok(FileResult::Content {
        content,
        binary,
        size,
        mtime: st.st_mtime,
    })
}

fn write(
    root: &Root,
    path: &str,
    content: &str,
    max_bytes: u32,
    expected_mtime: Option<i64>,
    create_only: bool,
) -> Out<FileResult> {
    if content.len() as u64 > u64::from(max_bytes.min(MAX_EDIT_BYTES)) {
        return fail(
            FileErrorCode::TooLarge,
            "the content is bigger than the limit",
        );
    }
    let (dir, name) = fs(root.parent(user_path(path)?))?;
    let mut mode = 0o644;
    match stat_at(&dir, &name) {
        Ok(st) => {
            if create_only {
                return fail(FileErrorCode::Exists, "already exists");
            }
            match kind_of(FileType::from_raw_mode(st.st_mode)) {
                EntryKind::File => {}
                EntryKind::Dir => return fail(FileErrorCode::IsDir, "is a directory"),
                _ => return fail(FileErrorCode::InvalidArguments, "not a regular file"),
            }
            if expected_mtime.is_some_and(|m| m != st.st_mtime) {
                return fail(FileErrorCode::Conflict, "the file changed meanwhile");
            }
            mode = st.st_mode & 0o777;
        }
        Err((FileErrorCode::NotFound, _)) => {
            if expected_mtime.is_some() {
                return fail(FileErrorCode::Conflict, "the file is gone");
            }
        }
        Err(other) => return Err(other),
    }
    let tmp = format!(".bp-tmp-{}", random_hex(8)?);
    let fd = sys(rustix::fs::openat(
        &dir,
        tmp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ))?;
    let written = (|| -> std::io::Result<()> {
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(mode))?;
        let mut file = std::fs::File::from(fd);
        file.write_all(content.as_bytes())?;
        file.sync_all()
    })();
    if written.is_err() {
        let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
        return fail(FileErrorCode::Failed, "cannot write");
    }
    let moved = if create_only {
        // Never over something that appeared meanwhile.
        rustix::fs::renameat_with(
            &dir,
            tmp.as_str(),
            &dir,
            name.as_str(),
            RenameFlags::NOREPLACE,
        )
    } else {
        rustix::fs::renameat(&dir, tmp.as_str(), &dir, name.as_str())
    };
    if let Err(e) = moved {
        let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
        return Err(from_fs(FsError::from(e)));
    }
    Ok(FileResult::Done { count: 1 })
}

fn mkdir(root: &Root, path: &str) -> Out<FileResult> {
    let (dir, name) = fs(root.parent(user_path(path)?))?;
    sys(rustix::fs::mkdirat(
        &dir,
        name.as_str(),
        Mode::from_raw_mode(0o755),
    ))?;
    Ok(FileResult::Done { count: 1 })
}

// ------------------------------------------------------------ move / copy

fn rename(root: &Root, from: &str, to: &str) -> Out<FileResult> {
    let (from_dir, from_name) = fs(root.parent(user_path(from)?))?;
    let (to_dir, to_name) = fs(root.parent(user_path(to)?))?;
    stat_at(&from_dir, &from_name)?;
    sys(rustix::fs::renameat_with(
        &from_dir,
        from_name.as_str(),
        &to_dir,
        to_name.as_str(),
        RenameFlags::NOREPLACE,
    ))?;
    Ok(FileResult::Done { count: 1 })
}

fn copy(root: &Root, from: &str, to: &str) -> Out<FileResult> {
    let (from_dir, from_name) = fs(root.parent(user_path(from)?))?;
    let (to_dir, to_name) = fs(root.parent(user_path(to)?))?;
    let st = stat_at(&from_dir, &from_name)?;
    // A directory cannot be copied into itself: the copy would never end.
    if kind_of(FileType::from_raw_mode(st.st_mode)) == EntryKind::Dir {
        let source = fs(builtpanel_safe_fs::parts(from).map_err(FsError::Path))?;
        let target = fs(builtpanel_safe_fs::parts(to).map_err(FsError::Path))?;
        if target.len() >= source.len() && target[..source.len()] == source[..] {
            return fail(
                FileErrorCode::InvalidArguments,
                "cannot copy a folder into itself",
            );
        }
    }
    let mut count = 0;
    copy_item(&from_dir, &from_name, &to_dir, &to_name, 0, &mut count)?;
    Ok(FileResult::Done { count })
}

fn copy_item(
    from_dir: &OwnedFd,
    from_name: &str,
    to_dir: &OwnedFd,
    to_name: &str,
    depth: usize,
    count: &mut u32,
) -> Out<()> {
    if depth > MAX_TREE_DEPTH || *count >= MAX_TREE_ITEMS {
        return fail(FileErrorCode::TooMany, "too many or too deep");
    }
    let st = stat_at(from_dir, from_name)?;
    match kind_of(FileType::from_raw_mode(st.st_mode)) {
        EntryKind::File => {
            let src = sys(rustix::fs::openat(
                from_dir,
                from_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
            let dst = sys(rustix::fs::openat(
                to_dir,
                to_name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(st.st_mode & 0o777),
            ))?;
            let mut src = std::fs::File::from(src);
            let mut dst = std::fs::File::from(dst);
            std::io::copy(&mut src, &mut dst)
                .or_else(|_| fail(FileErrorCode::Failed, "cannot copy"))?;
            *count += 1;
        }
        EntryKind::Dir => {
            sys(rustix::fs::mkdirat(
                to_dir,
                to_name,
                Mode::from_raw_mode(st.st_mode & 0o777),
            ))?;
            *count += 1;
            let src = sys(rustix::fs::openat(
                from_dir,
                from_name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
            let dst = sys(rustix::fs::openat(
                to_dir,
                to_name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
            for name in names(&src)? {
                copy_item(&src, &name, &dst, &name, depth + 1, count)?;
            }
        }
        // Links and special files are not copied (a link could point out of the tree).
        EntryKind::Link | EntryKind::Other => {}
    }
    Ok(())
}

// ------------------------------------------------------------ permissions

fn chmod(root: &Root, path: &str, mode: u32, recursive: bool) -> Out<FileResult> {
    if mode & !0o777 != 0 {
        return fail(
            FileErrorCode::InvalidMode,
            "only the nine permission bits may be set (no setuid, setgid or sticky bit)",
        );
    }
    let (dir, name) = fs(root.parent(user_path(path)?))?;
    let st = stat_at(&dir, &name)?;
    let is_dir = kind_of(FileType::from_raw_mode(st.st_mode)) == EntryKind::Dir;
    // The owner must keep the means to use it, or the tenant locks itself out.
    let needed = if is_dir { 0o700 } else { 0o600 };
    if mode & needed != needed {
        return fail(
            FileErrorCode::InvalidMode,
            "the owner must keep read and write (and, for a folder, search) rights",
        );
    }
    let mut count = 0;
    chmod_item(&dir, &name, mode, recursive, 0, &mut count)?;
    Ok(FileResult::Done { count })
}

fn chmod_item(
    dir: &OwnedFd,
    name: &str,
    mode: u32,
    recursive: bool,
    depth: usize,
    count: &mut u32,
) -> Out<()> {
    if depth > MAX_TREE_DEPTH || *count >= MAX_TREE_ITEMS {
        return fail(FileErrorCode::TooMany, "too many or too deep");
    }
    let st = stat_at(dir, name)?;
    let kind = kind_of(FileType::from_raw_mode(st.st_mode));
    if matches!(kind, EntryKind::Link | EntryKind::Other) {
        return Ok(());
    }
    let fd = sys(rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    sys(rustix::fs::fchmod(&fd, Mode::from_raw_mode(mode)))?;
    *count += 1;
    if recursive && kind == EntryKind::Dir {
        for child in names(&fd)? {
            chmod_item(&fd, &child, mode, true, depth + 1, count)?;
        }
    }
    Ok(())
}

fn fix_permissions(root: &Root, path: &str) -> Out<FileResult> {
    let at_root = fs(builtpanel_safe_fs::parts(path).map_err(FsError::Path))?.is_empty();
    if !at_root {
        user_path(path)?;
    }
    let mut count = 0;
    if at_root {
        let fd = fs(root.open_dir(""))?;
        fix_children(&fd, 0, &mut count, true)?;
        sys(rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o755)))?;
        count += 1;
    } else {
        let (dir, name) = fs(root.parent(path))?;
        fix_item(&dir, &name, 0, &mut count)?;
    }
    Ok(FileResult::Done { count })
}

fn fix_children(dir: &OwnedFd, depth: usize, count: &mut u32, top: bool) -> Out<()> {
    for name in names(dir)? {
        if top && name == TRASH_DIR {
            continue;
        }
        fix_item(dir, &name, depth + 1, count)?;
    }
    Ok(())
}

fn fix_item(dir: &OwnedFd, name: &str, depth: usize, count: &mut u32) -> Out<()> {
    if depth > MAX_TREE_DEPTH || *count >= MAX_TREE_ITEMS {
        return fail(FileErrorCode::TooMany, "too many or too deep");
    }
    let st = stat_at(dir, name)?;
    let kind = kind_of(FileType::from_raw_mode(st.st_mode));
    let wanted = match kind {
        EntryKind::Dir => 0o755,
        EntryKind::File if st.st_mode & 0o100 != 0 => 0o755,
        EntryKind::File => 0o644,
        EntryKind::Link | EntryKind::Other => return Ok(()),
    };
    let fd = sys(rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    sys(rustix::fs::fchmod(&fd, Mode::from_raw_mode(wanted)))?;
    *count += 1;
    if kind == EntryKind::Dir {
        fix_children(&fd, depth, count, false)?;
    }
    Ok(())
}

// ------------------------------------------------------------------- trash

mod trash {
    use super::{
        EntryKind, FileErrorCode, FileResult, FsError, Out, Root, TRASH_DIR, TrashItem, fail, fs,
        kind_of, now, random_hex, stat_at, sys, user_path,
    };
    use rustix::fs::{FileType, Mode, OFlags, RenameFlags};
    use std::{io::Read as _, io::Write as _, os::fd::OwnedFd};

    fn ensure(root: &Root) -> Out<OwnedFd> {
        match rustix::fs::mkdirat(root.fd(), TRASH_DIR, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(e) => return Err(super::from_fs(FsError::from(e))),
        }
        fs(root.open_dir(TRASH_DIR))
    }

    fn valid_id(id: &str) -> bool {
        id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    /// `meta`: `<unix seconds>\n<kind>\n<size>\n<original path>`.
    fn meta(deleted_at: i64, kind: EntryKind, size: u64, path: &str) -> String {
        let kind = match kind {
            EntryKind::Dir => "dir",
            EntryKind::File => "file",
            EntryKind::Link => "link",
            EntryKind::Other => "other",
        };
        format!("{deleted_at}\n{kind}\n{size}\n{path}")
    }

    fn parse_meta(id: &str, text: &str) -> Option<TrashItem> {
        let mut lines = text.splitn(4, '\n');
        let deleted_at = lines.next()?.parse().ok()?;
        let kind = match lines.next()? {
            "dir" => EntryKind::Dir,
            "file" => EntryKind::File,
            "link" => EntryKind::Link,
            _ => EntryKind::Other,
        };
        let size = lines.next()?.parse().ok()?;
        Some(TrashItem {
            id: id.to_owned(),
            path: lines.next()?.to_owned(),
            kind,
            size,
            deleted_at,
        })
    }

    pub(super) fn delete(root: &Root, path: &str) -> Out<FileResult> {
        let (dir, name) = fs(root.parent(user_path(path)?))?;
        let st = stat_at(&dir, &name)?;
        let kind = kind_of(FileType::from_raw_mode(st.st_mode));
        let size = u64::try_from(st.st_size).unwrap_or(0);
        let trash = ensure(root)?;
        let id = random_hex(16)?;
        sys(rustix::fs::mkdirat(
            &trash,
            id.as_str(),
            Mode::from_raw_mode(0o700),
        ))?;
        let item = fs(root.open_dir(&format!("{TRASH_DIR}/{id}")))?;
        let text = meta(now(), kind, size, path.trim_matches('/'));
        let wrote = (|| -> Out<()> {
            let fd = sys(rustix::fs::openat(
                &item,
                "meta",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ))?;
            std::fs::File::from(fd)
                .write_all(text.as_bytes())
                .or_else(|_| fail(FileErrorCode::Failed, "cannot write"))
        })();
        let moved =
            wrote.and_then(|()| sys(rustix::fs::renameat(&dir, name.as_str(), &item, "data")));
        if let Err(e) = moved {
            let _ = super::remove_tree(&trash, &id, 0, &mut 0);
            return Err(e);
        }
        Ok(FileResult::Done { count: 1 })
    }

    pub(super) fn list(root: &Root) -> Out<FileResult> {
        let trash = match fs(root.open_dir(TRASH_DIR)) {
            Ok(fd) => fd,
            Err((FileErrorCode::NotFound, _)) => return Ok(FileResult::Trash { items: vec![] }),
            Err(e) => return Err(e),
        };
        let mut items = Vec::new();
        for id in super::names(&trash)? {
            if !valid_id(&id) {
                continue;
            }
            let Ok(dir) = fs(root.open_dir(&format!("{TRASH_DIR}/{id}"))) else {
                continue;
            };
            let Ok(fd) = sys(rustix::fs::openat(
                &dir,
                "meta",
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )) else {
                continue;
            };
            let mut text = String::new();
            if std::fs::File::from(fd)
                .take(8192)
                .read_to_string(&mut text)
                .is_ok()
                && let Some(item) = parse_meta(&id, &text)
            {
                items.push(item);
            }
        }
        items.sort_by_key(|i| std::cmp::Reverse(i.deleted_at));
        Ok(FileResult::Trash { items })
    }

    pub(super) fn restore(root: &Root, id: &str) -> Out<FileResult> {
        if !valid_id(id) {
            return fail(FileErrorCode::InvalidArguments, "not a trash id");
        }
        let dir = fs(root.open_dir(&format!("{TRASH_DIR}/{id}")))?;
        let fd = sys(rustix::fs::openat(
            &dir,
            "meta",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let mut text = String::new();
        std::fs::File::from(fd)
            .take(8192)
            .read_to_string(&mut text)
            .or_else(|_| fail(FileErrorCode::Failed, "cannot read"))?;
        let item = parse_meta(id, &text).ok_or_else(|| {
            (
                FileErrorCode::Failed,
                "the trash entry is damaged".to_owned(),
            )
        })?;
        // Where it was; the folders above it are made again if they are gone.
        let parts = fs(builtpanel_safe_fs::parts(&item.path).map_err(FsError::Path))?;
        let (name, folders) = parts
            .split_last()
            .ok_or_else(|| (FileErrorCode::InvalidPath, "bad path".to_owned()))?;
        let mut so_far = String::new();
        for folder in folders {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(folder);
            let (parent, leaf) = fs(root.parent(&so_far))?;
            match rustix::fs::mkdirat(&parent, leaf.as_str(), Mode::from_raw_mode(0o755)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(super::from_fs(FsError::from(e))),
            }
        }
        let target = if folders.is_empty() {
            (*name).to_owned()
        } else {
            format!("{}/{name}", folders.join("/"))
        };
        let (to_dir, to_name) = fs(root.parent(&target))?;
        sys(rustix::fs::renameat_with(
            &dir,
            "data",
            &to_dir,
            to_name.as_str(),
            RenameFlags::NOREPLACE,
        ))?;
        let trash = fs(root.open_dir(TRASH_DIR))?;
        super::remove_tree(&trash, id, 0, &mut 0)?;
        Ok(FileResult::Done { count: 1 })
    }

    pub(super) fn empty(
        root: &Root,
        id: Option<&str>,
        older_than_days: Option<u32>,
    ) -> Out<FileResult> {
        let trash = match fs(root.open_dir(TRASH_DIR)) {
            Ok(fd) => fd,
            Err((FileErrorCode::NotFound, _)) => return Ok(FileResult::Done { count: 0 }),
            Err(e) => return Err(e),
        };
        let mut removed = 0;
        let FileResult::Trash { items } = list(root)? else {
            return Ok(FileResult::Done { count: 0 });
        };
        let limit = older_than_days.map(|d| now() - i64::from(d) * 86_400);
        for item in items {
            if id.is_some_and(|wanted| wanted != item.id) {
                continue;
            }
            if limit.is_some_and(|l| item.deleted_at > l) {
                continue;
            }
            super::remove_tree(&trash, &item.id, 0, &mut 0)?;
            removed += 1;
        }
        Ok(FileResult::Done { count: removed })
    }
}

/// Removes `name` of `dir` and everything below it, following no link.
fn remove_tree(dir: &OwnedFd, name: &str, depth: usize, count: &mut u32) -> Out<()> {
    if depth > MAX_TREE_DEPTH + 8 {
        return fail(FileErrorCode::TooMany, "too deep");
    }
    let st = stat_at(dir, name)?;
    if kind_of(FileType::from_raw_mode(st.st_mode)) == EntryKind::Dir {
        let fd = sys(rustix::fs::openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        // A directory without the right to enter cannot be emptied: make it so.
        let _ = rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o700));
        for child in names(&fd)? {
            remove_tree(&fd, &child, depth + 1, count)?;
        }
        fs(builtpanel_safe_fs::remove(dir, name, true))?;
    } else {
        fs(builtpanel_safe_fs::remove(dir, name, false))?;
    }
    *count += 1;
    Ok(())
}

// ---------------------------------------------------------------- transfers

/// Puts the bytes of `input` (at most `max_bytes`) into the file `dest`, made
/// beside its place and moved in at the end, so a cut transfer leaves nothing
/// half done. The folders above `dest` are made when missing. Never over something
/// unless `overwrite`. Returns the number of bytes.
///
/// # Errors
/// The usual refusals of the file operations; [`FileErrorCode::TooLarge`] when `input`
/// has more than `max_bytes`.
pub fn place(
    root: &Root,
    dest: &str,
    overwrite: bool,
    max_bytes: u64,
    input: &mut dyn std::io::Read,
) -> Out<u64> {
    let parts = fs(builtpanel_safe_fs::parts(user_path(dest)?).map_err(FsError::Path))?;
    let (name, folders) = parts.split_last().ok_or_else(|| {
        (
            FileErrorCode::InvalidPath,
            "the path names nothing".to_owned(),
        )
    })?;
    let mut so_far = String::new();
    for folder in folders {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(folder);
        let (parent, leaf) = fs(root.parent(&so_far))?;
        match rustix::fs::mkdirat(&parent, leaf.as_str(), Mode::from_raw_mode(0o755)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(e) => return Err(from_fs(FsError::from(e))),
        }
    }
    let (dir, name) = fs(root.parent(&if folders.is_empty() {
        (*name).to_owned()
    } else {
        format!("{}/{name}", folders.join("/"))
    }))?;
    match stat_at(&dir, &name) {
        Ok(st) => {
            if !overwrite {
                return fail(FileErrorCode::Exists, "already exists");
            }
            if kind_of(FileType::from_raw_mode(st.st_mode)) != EntryKind::File {
                return fail(FileErrorCode::IsDir, "it is not a file");
            }
        }
        Err((FileErrorCode::NotFound, _)) => {}
        Err(other) => return Err(other),
    }
    let tmp = format!(".bp-up-{}", random_hex(8)?);
    let fd = sys(rustix::fs::openat(
        &dir,
        tmp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ))?;
    let copied = (|| -> std::io::Result<u64> {
        let mut file = std::fs::File::from(fd);
        let n = std::io::copy(&mut input.take(max_bytes.saturating_add(1)), &mut file)?;
        if n <= max_bytes {
            rustix::fs::fchmod(&file, Mode::from_raw_mode(0o644))?;
            file.sync_all()?;
        }
        Ok(n)
    })();
    let size = match copied {
        Ok(n) if n <= max_bytes => n,
        Ok(_) => {
            let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
            return fail(FileErrorCode::TooLarge, "the file is bigger than the limit");
        }
        Err(e) => {
            let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
            // A full disk or a quota shows here.
            return Err((
                FileErrorCode::Failed,
                format!("cannot write the file: {}", e.kind()),
            ));
        }
    };
    let moved = if overwrite {
        rustix::fs::renameat(&dir, tmp.as_str(), &dir, name.as_str())
    } else {
        rustix::fs::renameat_with(
            &dir,
            tmp.as_str(),
            &dir,
            name.as_str(),
            RenameFlags::NOREPLACE,
        )
    };
    if let Err(e) = moved {
        let _ = builtpanel_safe_fs::remove(&dir, &tmp, false);
        return Err(from_fs(FsError::from(e)));
    }
    Ok(size)
}

/// Copies the file `path` (at most `max_bytes`) into `out`. Returns the number of bytes.
///
/// # Errors
/// The usual refusals; [`FileErrorCode::TooLarge`] for a bigger file.
pub fn take(root: &Root, path: &str, max_bytes: u64, out: &mut dyn std::io::Write) -> Out<u64> {
    let fd = fs(root.open_at(
        user_path(path)?,
        OFlags::RDONLY | OFlags::NONBLOCK,
        Mode::empty(),
    ))?;
    let st = sys(fstat(&fd))?;
    match kind_of(FileType::from_raw_mode(st.st_mode)) {
        EntryKind::File => {}
        EntryKind::Dir => return fail(FileErrorCode::IsDir, "is a directory"),
        _ => return fail(FileErrorCode::InvalidArguments, "not a regular file"),
    }
    if u64::try_from(st.st_size).unwrap_or(0) > max_bytes {
        return fail(FileErrorCode::TooLarge, "the file is bigger than the limit");
    }
    let n = std::io::copy(
        &mut std::fs::File::from(fd).take(max_bytes.saturating_add(1)),
        out,
    )
    .or_else(|_| fail(FileErrorCode::Failed, "cannot read the file"))?;
    if n > max_bytes {
        return fail(FileErrorCode::TooLarge, "the file is bigger than the limit");
    }
    Ok(n)
}

/// A failure of [`place`] or [`take`] as an answer.
#[must_use]
pub fn failure(code: FileErrorCode, message: String) -> FileResult {
    FileResult::Error { code, message }
}
