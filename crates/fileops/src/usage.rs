//! What a tenant's files use (T08.10): disk and file count, per folder.
//!
//! One walk over everything below the root, never following a link, counting the
//! blocks the disk really holds (`st_blocks`), so sparse files and the like are
//! seen as they are. The walk stops at [`MAX_ENTRIES`] and says so.

use std::os::fd::OwnedFd;

use builtpanel_node_protocol::files::{FileErrorCode, FileResult, FolderUsage};
use rustix::fs::{FileType, Mode, OFlags};

use super::{EntryKind, Out, fail, kind_of, names, stat_at, sys};

/// Most entries one walk looks at.
const MAX_ENTRIES: u64 = 5_000_000;
/// Deepest folder named in the answer.
const MAX_NAMED_DEPTH: usize = 8;
const MAX_DEPTH: usize = 128;

struct Walk {
    seen: u64,
    partial: bool,
    folders: Vec<FolderUsage>,
}

pub(super) fn disk_usage(root: &builtpanel_safe_fs::Root, top: u32) -> Out<FileResult> {
    if !(1..=100).contains(&top) {
        return fail(FileErrorCode::InvalidArguments, "name 1 to 100 folders");
    }
    let dir = root
        .open_dir("")
        .map_err(|_| (FileErrorCode::Failed, "cannot open the files".to_owned()))?;
    let mut walk = Walk {
        seen: 0,
        partial: false,
        folders: Vec::new(),
    };
    let (bytes, inodes) = below(&dir, "", 0, &mut walk)?;
    walk.folders
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    walk.folders.truncate(top as usize);
    Ok(FileResult::Usage {
        total_bytes: bytes,
        inodes,
        folders: walk.folders,
        partial: walk.partial,
    })
}

/// Bytes and inodes of everything in `dir` (a folder already open), `path` being
/// its name for the answer.
fn below(dir: &OwnedFd, path: &str, depth: usize, walk: &mut Walk) -> Out<(u64, u64)> {
    let (mut bytes, mut inodes) = (0u64, 0u64);
    if depth > MAX_DEPTH {
        walk.partial = true;
        return Ok((0, 0));
    }
    for name in names(dir)? {
        if walk.seen >= MAX_ENTRIES {
            walk.partial = true;
            break;
        }
        walk.seen += 1;
        // Gone since it was listed: skipped.
        let Ok(st) = stat_at(dir, &name) else {
            continue;
        };
        let blocks = u64::try_from(st.st_blocks).unwrap_or(0).saturating_mul(512);
        bytes = bytes.saturating_add(blocks);
        inodes += 1;
        if kind_of(FileType::from_raw_mode(st.st_mode)) == EntryKind::Dir {
            let Ok(fd) = sys(rustix::fs::openat(
                dir,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )) else {
                continue;
            };
            let child = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let (b, i) = below(&fd, &child, depth + 1, walk)?;
            bytes = bytes.saturating_add(b);
            inodes += i;
            if depth < MAX_NAMED_DEPTH {
                walk.folders.push(FolderUsage {
                    path: child,
                    bytes: blocks.saturating_add(b),
                    inodes: i + 1,
                });
            }
        }
    }
    Ok((bytes, inodes))
}
