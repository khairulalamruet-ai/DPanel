//! The file operations on a real directory.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink},
    path::PathBuf,
};

use builtpanel_fileops::handle;
use builtpanel_node_protocol::files::{EntryKind, FileErrorCode, FileRequest as R, FileResult};
use builtpanel_safe_fs::Root;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bp-fileops-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tenant")).unwrap();
        Self(dir)
    }
    fn root(&self) -> Root {
        Root::open(&self.0.join("tenant")).unwrap()
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join("tenant").join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn err(r: FileResult) -> FileErrorCode {
    match r {
        FileResult::Error { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn done(r: FileResult) -> u32 {
    match r {
        FileResult::Done { count } => count,
        other => panic!("expected done, got {other:?}"),
    }
}

fn write(path: &str, content: &str) -> R {
    R::Write {
        path: path.into(),
        content: content.into(),
        max_bytes: 1000,
        expected_mtime: None,
        create_only: false,
    }
}

#[test]
fn listing_is_paged_sorted_folders_first_and_hides_the_trash() {
    let s = Scratch::new("list");
    let root = s.root();
    for n in ["b.txt", "a.txt", "c.txt", "e.txt", "d.txt"] {
        std::fs::write(s.path(n), n).unwrap();
    }
    std::fs::create_dir(s.path("zdir")).unwrap();
    std::fs::create_dir(s.path(".trash")).unwrap();
    symlink("/etc", s.path("alink")).unwrap();
    let page = |offset, limit| match handle(
        &root,
        R::List {
            path: String::new(),
            offset,
            limit,
        },
    ) {
        FileResult::Entries { entries, total } => (
            entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
            total,
            entries,
        ),
        other => panic!("{other:?}"),
    };
    let (names, total, entries) = page(0, 3);
    assert_eq!(
        (names.as_slice(), total),
        (["zdir", "a.txt", "alink"].map(String::from).as_slice(), 7)
    );
    assert_eq!(entries[0].kind, EntryKind::Dir);
    assert_eq!(
        entries[2].kind,
        EntryKind::Link,
        "a link is shown as a link, never followed"
    );
    assert_eq!(page(3, 3).0, ["b.txt", "c.txt", "d.txt"]);
    assert_eq!(page(6, 3).0, ["e.txt"]);
    assert!(page(0, 1000).0.iter().all(|n| n != ".trash"));
    assert_eq!(
        err(handle(
            &root,
            R::List {
                path: String::new(),
                offset: 0,
                limit: 0
            }
        )),
        FileErrorCode::InvalidArguments
    );
    assert_eq!(
        err(handle(
            &root,
            R::List {
                path: String::new(),
                offset: 0,
                limit: 5000
            }
        )),
        FileErrorCode::InvalidArguments
    );
    // Through a link, out of the root, into the trash, into a file: refused.
    for (path, code) in [
        ("alink", FileErrorCode::Link),
        ("../x", FileErrorCode::InvalidPath),
        (".trash", FileErrorCode::InvalidPath),
        ("a.txt", FileErrorCode::NotDir),
        ("missing", FileErrorCode::NotFound),
    ] {
        let r = handle(
            &root,
            R::List {
                path: path.into(),
                offset: 0,
                limit: 10,
            },
        );
        let got = err(r);
        assert!(
            got == code || (code == FileErrorCode::Link && got == FileErrorCode::NotDir),
            "{path}: {got:?}"
        );
    }
    match handle(
        &root,
        R::Stat {
            path: "a.txt".into(),
        },
    ) {
        FileResult::Entry { entry } => {
            assert_eq!((entry.kind, entry.size), (EntryKind::File, 5));
            assert_eq!(entry.mode & 0o600, 0o600);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        err(handle(
            &root,
            R::Stat {
                path: "nope".into()
            }
        )),
        FileErrorCode::NotFound
    );
}

#[test]
fn a_file_is_read_and_written_whole_within_the_limit() {
    let s = Scratch::new("rw");
    let root = s.root();
    assert_eq!(done(handle(&root, write("note.txt", "hello"))), 1);
    let (mtime, size) = match handle(
        &root,
        R::Read {
            path: "note.txt".into(),
            max_bytes: 100,
        },
    ) {
        FileResult::Content {
            content,
            binary,
            size,
            mtime,
        } => {
            assert_eq!((content.as_deref(), binary), (Some("hello"), false));
            (mtime, size)
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(size, 5);
    // Too big, in both directions.
    assert_eq!(
        err(handle(
            &root,
            R::Read {
                path: "note.txt".into(),
                max_bytes: 4
            }
        )),
        FileErrorCode::TooLarge
    );
    let big = "x".repeat(1001);
    assert_eq!(
        err(handle(&root, write("big.txt", &big))),
        FileErrorCode::TooLarge
    );
    assert!(!s.path("big.txt").exists());
    // A file that changed meanwhile is not overwritten blindly.
    let mut w = write("note.txt", "second");
    if let R::Write { expected_mtime, .. } = &mut w {
        *expected_mtime = Some(mtime - 100);
    }
    assert_eq!(err(handle(&root, w)), FileErrorCode::Conflict);
    let mut w = write("note.txt", "second");
    if let R::Write { expected_mtime, .. } = &mut w {
        *expected_mtime = Some(mtime);
    }
    assert_eq!(done(handle(&root, w)), 1);
    assert_eq!(
        std::fs::read_to_string(s.path("note.txt")).unwrap(),
        "second"
    );
    // create_only never replaces; the mode of an existing file is kept; no temp file is left.
    std::fs::set_permissions(s.path("note.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut w = write("note.txt", "third");
    if let R::Write { create_only, .. } = &mut w {
        *create_only = true;
    }
    assert_eq!(err(handle(&root, w)), FileErrorCode::Exists);
    assert_eq!(done(handle(&root, write("note.txt", "third"))), 1);
    assert_eq!(
        std::fs::metadata(s.path("note.txt")).unwrap().mode() & 0o777,
        0o640
    );
    let leftovers: Vec<_> = std::fs::read_dir(s.path(""))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["note.txt"]);
    // Binary files are not text; directories and links are not files.
    std::fs::write(s.path("bin"), [0xff, 0xfe, 0x00]).unwrap();
    match handle(
        &root,
        R::Read {
            path: "bin".into(),
            max_bytes: 100,
        },
    ) {
        FileResult::Content {
            content, binary, ..
        } => assert_eq!((content, binary), (None, true)),
        other => panic!("{other:?}"),
    }
    std::fs::create_dir(s.path("d")).unwrap();
    assert_eq!(
        err(handle(
            &root,
            R::Read {
                path: "d".into(),
                max_bytes: 100
            }
        )),
        FileErrorCode::IsDir
    );
    assert_eq!(err(handle(&root, write("d", "x"))), FileErrorCode::IsDir);
    // Writing through a link replaces nothing outside.
    std::fs::write(s.0.join("outside.txt"), "outside").unwrap();
    symlink(s.0.join("outside.txt"), s.path("l")).unwrap();
    let r = handle(
        &root,
        R::Read {
            path: "l".into(),
            max_bytes: 100,
        },
    );
    assert!(matches!(
        err(r),
        FileErrorCode::Link | FileErrorCode::NotDir
    ));
    assert_eq!(
        std::fs::read_to_string(s.0.join("outside.txt")).unwrap(),
        "outside"
    );
    assert_eq!(
        err(handle(&root, write("../outside.txt", "pwn"))),
        FileErrorCode::InvalidPath
    );
    assert_eq!(
        err(handle(&root, write(".trash/x", "pwn"))),
        FileErrorCode::InvalidPath
    );
}

#[test]
fn move_and_copy_never_replace_and_copy_leaves_links_behind() {
    let s = Scratch::new("mv");
    let root = s.root();
    std::fs::create_dir_all(s.path("site/sub")).unwrap();
    std::fs::write(s.path("site/a.txt"), "a").unwrap();
    std::fs::write(s.path("site/sub/b.txt"), "b").unwrap();
    symlink("/etc/passwd", s.path("site/passwd")).unwrap();
    assert_eq!(done(handle(&root, R::Mkdir { path: "new".into() })), 1);
    assert_eq!(
        err(handle(&root, R::Mkdir { path: "new".into() })),
        FileErrorCode::Exists
    );
    // A copy of a whole folder: files and folders, no link.
    assert_eq!(
        done(handle(
            &root,
            R::Copy {
                from: "site".into(),
                to: "new/copy".into()
            }
        )),
        4
    );
    assert_eq!(
        std::fs::read_to_string(s.path("new/copy/sub/b.txt")).unwrap(),
        "b"
    );
    assert!(!s.path("new/copy/passwd").exists());
    assert_eq!(
        err(handle(
            &root,
            R::Copy {
                from: "site".into(),
                to: "new/copy".into()
            }
        )),
        FileErrorCode::Exists
    );
    assert_eq!(
        err(handle(
            &root,
            R::Copy {
                from: "site".into(),
                to: "site/sub/in".into()
            }
        )),
        FileErrorCode::InvalidArguments
    );
    // Move: never over something.
    assert_eq!(
        done(handle(
            &root,
            R::Move {
                from: "site/a.txt".into(),
                to: "moved.txt".into()
            }
        )),
        1
    );
    assert!(!s.path("site/a.txt").exists());
    assert_eq!(
        err(handle(
            &root,
            R::Move {
                from: "site/sub/b.txt".into(),
                to: "moved.txt".into()
            }
        )),
        FileErrorCode::Exists
    );
    assert_eq!(std::fs::read_to_string(s.path("moved.txt")).unwrap(), "a");
    assert_eq!(
        err(handle(
            &root,
            R::Move {
                from: "nothing".into(),
                to: "x".into()
            }
        )),
        FileErrorCode::NotFound
    );
    assert_eq!(
        err(handle(
            &root,
            R::Move {
                from: "moved.txt".into(),
                to: "../out".into()
            }
        )),
        FileErrorCode::InvalidPath
    );
    // A folder cannot move into itself.
    assert!(matches!(
        handle(
            &root,
            R::Move {
                from: "site".into(),
                to: "site/sub/x".into()
            }
        ),
        FileResult::Error { .. }
    ));
}

#[test]
fn deleting_goes_to_the_trash_and_comes_back() {
    let s = Scratch::new("trash");
    let root = s.root();
    std::fs::create_dir_all(s.path("www/shop")).unwrap();
    std::fs::write(s.path("www/shop/index.php"), "<?php").unwrap();
    std::fs::write(s.path("www/old.txt"), "old").unwrap();
    assert_eq!(
        done(handle(
            &root,
            R::Delete {
                path: "www/shop".into()
            }
        )),
        1
    );
    assert_eq!(
        done(handle(
            &root,
            R::Delete {
                path: "www/old.txt".into()
            }
        )),
        1
    );
    assert!(!s.path("www/shop").exists());
    let FileResult::Trash { items } = handle(&root, R::TrashList) else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    let shop = items.iter().find(|i| i.path == "www/shop").unwrap();
    assert_eq!(shop.kind, EntryKind::Dir);
    // Back to where it was; if something is there now, no overwrite.
    assert_eq!(
        done(handle(
            &root,
            R::Restore {
                id: shop.id.clone()
            }
        )),
        1
    );
    assert_eq!(
        std::fs::read_to_string(s.path("www/shop/index.php")).unwrap(),
        "<?php"
    );
    let old = items.iter().find(|i| i.path == "www/old.txt").unwrap();
    std::fs::write(s.path("www/old.txt"), "new one").unwrap();
    assert_eq!(
        err(handle(&root, R::Restore { id: old.id.clone() })),
        FileErrorCode::Exists
    );
    std::fs::remove_file(s.path("www/old.txt")).unwrap();
    // Even when its folders are gone.
    std::fs::remove_dir_all(s.path("www")).unwrap();
    assert_eq!(done(handle(&root, R::Restore { id: old.id.clone() })), 1);
    assert_eq!(
        std::fs::read_to_string(s.path("www/old.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        err(handle(
            &root,
            R::Restore {
                id: "../../etc".into()
            }
        )),
        FileErrorCode::InvalidArguments
    );
    // Emptying: one item, then everything.
    let _ = handle(
        &root,
        R::Delete {
            path: "www/old.txt".into(),
        },
    );
    std::fs::write(s.path("www/second.txt"), "2").unwrap();
    let _ = handle(
        &root,
        R::Delete {
            path: "www/second.txt".into(),
        },
    );
    let FileResult::Trash { items } = handle(&root, R::TrashList) else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert_eq!(
        done(handle(
            &root,
            R::EmptyTrash {
                id: Some(items[0].id.clone()),
                older_than_days: None
            }
        )),
        1
    );
    assert_eq!(
        done(handle(
            &root,
            R::EmptyTrash {
                id: None,
                older_than_days: Some(1)
            }
        )),
        0,
        "nothing is a day old yet"
    );
    assert_eq!(
        done(handle(
            &root,
            R::EmptyTrash {
                id: None,
                older_than_days: None
            }
        )),
        1
    );
    let FileResult::Trash { items } = handle(&root, R::TrashList) else {
        panic!()
    };
    assert!(items.is_empty());
    // The trash is not a path the tenant can use directly.
    assert_eq!(
        err(handle(
            &root,
            R::Delete {
                path: ".trash".into()
            }
        )),
        FileErrorCode::InvalidPath
    );
    assert_eq!(
        err(handle(
            &root,
            R::Move {
                from: ".trash".into(),
                to: "x".into()
            }
        )),
        FileErrorCode::InvalidPath
    );
}

#[test]
fn permissions_cannot_gain_setuid_or_lock_the_owner_out() {
    let s = Scratch::new("mode");
    let root = s.root();
    std::fs::create_dir_all(s.path("d/e")).unwrap();
    std::fs::write(s.path("d/f.sh"), "#!/bin/sh").unwrap();
    std::fs::write(s.path("d/e/g.txt"), "g").unwrap();
    let mode = |p: &str| std::fs::metadata(s.path(p)).unwrap().mode() & 0o7777;
    for bad in [0o4755, 0o2755, 0o1777, 0o6777, 0o10_000] {
        assert_eq!(
            err(handle(
                &root,
                R::Chmod {
                    path: "d/f.sh".into(),
                    mode: bad,
                    recursive: false
                }
            )),
            FileErrorCode::InvalidMode,
            "{bad:o}"
        );
    }
    assert_eq!(
        err(handle(
            &root,
            R::Chmod {
                path: "d/f.sh".into(),
                mode: 0o044,
                recursive: false
            }
        )),
        FileErrorCode::InvalidMode
    );
    assert_eq!(
        err(handle(
            &root,
            R::Chmod {
                path: "d".into(),
                mode: 0o666,
                recursive: false
            }
        )),
        FileErrorCode::InvalidMode
    );
    assert_eq!(
        done(handle(
            &root,
            R::Chmod {
                path: "d/f.sh".into(),
                mode: 0o755,
                recursive: false
            }
        )),
        1
    );
    assert_eq!(mode("d/f.sh"), 0o755);
    assert_eq!(
        done(handle(
            &root,
            R::Chmod {
                path: "d".into(),
                mode: 0o750,
                recursive: true
            }
        )),
        4
    );
    assert_eq!(
        (mode("d"), mode("d/e"), mode("d/e/g.txt")),
        (0o750, 0o750, 0o750)
    );
    // Fixing: folders 755, files 644, scripts the owner could run 755; links untouched.
    std::fs::set_permissions(s.path("d/e/g.txt"), std::fs::Permissions::from_mode(0o666)).unwrap();
    symlink("/etc", s.path("d/lnk")).unwrap();
    assert_eq!(
        done(handle(&root, R::FixPermissions { path: "d".into() })),
        4
    );
    assert_eq!(
        (mode("d"), mode("d/e"), mode("d/e/g.txt"), mode("d/f.sh")),
        (0o755, 0o755, 0o644, 0o755)
    );
    assert_eq!(
        done(handle(
            &root,
            R::FixPermissions {
                path: String::new()
            }
        )),
        5
    );
    assert_eq!(
        err(handle(
            &root,
            R::Chmod {
                path: "nothing".into(),
                mode: 0o644,
                recursive: false
            }
        )),
        FileErrorCode::NotFound
    );
    assert_eq!(
        done(handle(
            &root,
            R::Chmod {
                path: "d/lnk".into(),
                mode: 0o644,
                recursive: false
            }
        )),
        0,
        "a link is left alone"
    );
}

#[test]
fn disk_usage_counts_blocks_and_files_and_names_the_biggest_folders() {
    let s = Scratch::new("usage");
    let root = s.root();
    std::fs::create_dir_all(s.path("www/big")).unwrap();
    std::fs::create_dir_all(s.path("www/small")).unwrap();
    std::fs::create_dir_all(s.path("logs")).unwrap();
    std::fs::write(s.path("www/big/a.bin"), vec![1u8; 300_000]).unwrap();
    std::fs::write(s.path("www/big/b.bin"), vec![2u8; 200_000]).unwrap();
    std::fs::write(s.path("www/small/c.txt"), "c").unwrap();
    std::fs::write(s.path("logs/x.log"), vec![3u8; 100_000]).unwrap();
    symlink("/etc", s.path("link")).unwrap();
    let FileResult::Usage {
        total_bytes,
        inodes,
        folders,
        partial,
    } = handle(&root, R::DiskUsage { top: 3 })
    else {
        panic!()
    };
    assert!(!partial);
    // 3 folders at the top + big, small + 4 files + the link = 9
    assert_eq!(inodes, 9);
    assert!((600_000..700_000).contains(&total_bytes), "{total_bytes}");
    let names: Vec<_> = folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["www", "www/big", "logs"], "{folders:?}");
    assert!(folders[0].bytes >= 500_000 && folders[1].bytes >= 500_000);
    assert_eq!(folders[1].inodes, 3);
    assert_eq!(
        err(handle(&root, R::DiskUsage { top: 0 })),
        FileErrorCode::InvalidArguments
    );
}
