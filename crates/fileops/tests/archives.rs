//! Packing and unpacking on a real directory, with hostile archives.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::Write as _,
    os::unix::fs::{MetadataExt as _, symlink},
    path::PathBuf,
};

use builtpanel_fileops::handle;
use builtpanel_node_protocol::files::{ArchiveFormat, FileErrorCode, FileRequest as R, FileResult};
use builtpanel_safe_fs::Root;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bp-arch-{tag}-{}", std::process::id()));
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

fn code(r: FileResult) -> FileErrorCode {
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

fn extract(root: &Root, path: &str, dest: &str) -> FileResult {
    handle(
        root,
        R::Extract {
            path: path.into(),
            dest: dest.into(),
            max_bytes: 1 << 20,
            max_entries: 100,
        },
    )
}

fn zip_with(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data, mode) in entries {
        let mut options = zip::write::SimpleFileOptions::default();
        if let Some(m) = mode {
            options = options.unix_permissions(*m);
        }
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A tar.gz with entries written byte by byte, so names the library would refuse
/// can be tried.
fn tar_gz_with(entries: &[(&str, tar::EntryType, &[u8], u32, &str)]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (name, kind, data, mode, link) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(data.len() as u64);
        header.set_mode(*mode);
        {
            let old = header.as_old_mut();
            old.name[..name.len()].copy_from_slice(name.as_bytes());
            old.linkname[..link.len()].copy_from_slice(link.as_bytes());
        }
        header.set_cksum();
        raw.extend_from_slice(header.as_bytes());
        raw.extend_from_slice(data);
        raw.resize(raw.len().div_ceil(512) * 512, 0);
    }
    raw.extend_from_slice(&[0u8; 1024]);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

#[test]
fn what_is_packed_comes_out_the_same_in_both_formats() {
    for (format, ext) in [
        (ArchiveFormat::Zip, "zip"),
        (ArchiveFormat::TarGz, "tar.gz"),
    ] {
        let s = Scratch::new(ext);
        let root = s.root();
        std::fs::create_dir_all(s.path("site/sub")).unwrap();
        std::fs::write(s.path("site/a.txt"), "alpha").unwrap();
        std::fs::write(s.path("site/sub/b.txt"), "beta").unwrap();
        std::fs::write(s.path("single.txt"), "one").unwrap();
        symlink("/etc/passwd", s.path("site/link")).unwrap();
        let pack = |dest: &str, max_bytes| {
            handle(
                &root,
                R::Archive {
                    paths: vec!["site".into(), "single.txt".into()],
                    dest: dest.into(),
                    format,
                    max_bytes,
                },
            )
        };
        assert_eq!(done(pack(&format!("out.{ext}"), 1 << 20)), 5, "{ext}");
        assert_eq!(
            code(pack(&format!("out.{ext}"), 1 << 20)),
            FileErrorCode::Exists
        );
        assert_eq!(
            code(pack(&format!("small.{ext}"), 3)),
            FileErrorCode::TooLarge
        );
        assert!(
            !s.path(&format!("small.{ext}")).exists(),
            "no half archive is left"
        );
        assert_eq!(
            done(extract(&root, &format!("out.{ext}"), "back")),
            5,
            "{ext}"
        );
        assert_eq!(
            std::fs::read_to_string(s.path("back/site/a.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            std::fs::read_to_string(s.path("back/site/sub/b.txt")).unwrap(),
            "beta"
        );
        assert_eq!(
            std::fs::read_to_string(s.path("back/single.txt")).unwrap(),
            "one"
        );
        assert!(!s.path("back/site/link").exists(), "links are not packed");
        // Nothing is replaced: unpacking again says so.
        assert_eq!(
            code(extract(&root, &format!("out.{ext}"), "back")),
            FileErrorCode::Exists
        );
    }
}

#[test]
fn an_archive_that_would_leave_the_folder_is_refused_before_anything_is_written() {
    let s = Scratch::new("slip");
    let root = s.root();
    std::fs::write(s.0.join("victim"), "keep").unwrap();
    for (name, bytes) in [
        (
            "evil1.zip",
            zip_with(&[("fine.txt", b"x", None), ("../../etc/passwd", b"pwn", None)]),
        ),
        ("evil2.zip", zip_with(&[("/etc/passwd", b"pwn", None)])),
        ("evil3.zip", zip_with(&[("a/../../victim", b"pwn", None)])),
        ("evil4.zip", zip_with(&[("a\\..\\b", b"pwn", None)])),
        (
            "evil5.tar.gz",
            tar_gz_with(&[
                ("fine.txt", tar::EntryType::Regular, b"x", 0o644, ""),
                ("../victim", tar::EntryType::Regular, b"pwn", 0o644, ""),
            ]),
        ),
        (
            "evil6.tar.gz",
            tar_gz_with(&[("/etc/cron.d/x", tar::EntryType::Regular, b"pwn", 0o644, "")]),
        ),
    ] {
        std::fs::write(s.path(name), bytes).unwrap();
        let dest = format!("into-{name}");
        assert_eq!(
            code(extract(&root, name, &dest)),
            FileErrorCode::BadArchive,
            "{name}"
        );
        assert!(
            !s.path(&dest).exists(),
            "{name}: not even the folder was made"
        );
        assert!(!s.path("fine.txt").exists() && !s.path(&format!("{dest}/fine.txt")).exists());
    }
    assert_eq!(std::fs::read_to_string(s.0.join("victim")).unwrap(), "keep");
    // Not an archive at all, or damaged.
    std::fs::write(s.path("text.zip"), "just text").unwrap();
    assert_eq!(
        code(extract(&root, "text.zip", "t")),
        FileErrorCode::BadArchive
    );
    let mut cut = zip_with(&[("a.txt", &[7u8; 5000], None)]);
    cut.truncate(cut.len() / 2);
    std::fs::write(s.path("cut.zip"), cut).unwrap();
    assert_eq!(
        code(extract(&root, "cut.zip", "c")),
        FileErrorCode::BadArchive
    );
}

#[test]
fn links_are_skipped_modes_are_cut_and_limits_hold() {
    let s = Scratch::new("limits");
    let root = s.root();
    let tgz = tar_gz_with(&[
        ("dir/", tar::EntryType::Directory, b"", 0o000, ""),
        (
            "dir/run.sh",
            tar::EntryType::Regular,
            b"#!/bin/sh",
            0o4755,
            "",
        ),
        ("dir/secret", tar::EntryType::Regular, b"s", 0o000, ""),
        (
            "dir/link",
            tar::EntryType::Symlink,
            b"",
            0o777,
            "/etc/passwd",
        ),
        ("dir/hard", tar::EntryType::Link, b"", 0o777, "dir/run.sh"),
    ]);
    std::fs::write(s.path("a.tgz"), tgz).unwrap();
    assert_eq!(
        done(extract(&root, "a.tgz", "out")),
        3,
        "two files and a folder; the links are skipped"
    );
    assert!(!s.path("out/dir/link").exists() && !s.path("out/dir/hard").exists());
    let mode = |p: &str| std::fs::metadata(s.path(p)).unwrap().mode() & 0o7777;
    assert_eq!(mode("out/dir/run.sh"), 0o755, "no setuid bit");
    assert_eq!(
        mode("out/dir/secret") & 0o600,
        0o600,
        "the owner can always use it"
    );
    assert_eq!(mode("out/dir") & 0o700, 0o700);
    // A zip that is bigger than the limit, or has too many entries.
    std::fs::write(
        s.path("big.zip"),
        zip_with(&[("a", &vec![0u8; 5000], None)]),
    )
    .unwrap();
    let r = handle(
        &root,
        R::Extract {
            path: "big.zip".into(),
            dest: "b".into(),
            max_bytes: 4000,
            max_entries: 10,
        },
    );
    assert_eq!(code(r), FileErrorCode::TooLarge);
    std::fs::write(
        s.path("many.zip"),
        zip_with(&[("a", b"1", None), ("b", b"2", None), ("c", b"3", None)]),
    )
    .unwrap();
    let r = handle(
        &root,
        R::Extract {
            path: "many.zip".into(),
            dest: "m".into(),
            max_bytes: 100,
            max_entries: 2,
        },
    );
    assert_eq!(code(r), FileErrorCode::TooMany);
    assert!(!s.path("b").exists() && !s.path("m").exists());
    // Into a link, or out of the root: refused.
    symlink("/tmp", s.path("tmp-link")).unwrap();
    assert!(matches!(
        code(extract(&root, "a.tgz", "tmp-link/x")),
        FileErrorCode::Link | FileErrorCode::NotDir
    ));
    assert_eq!(
        code(extract(&root, "a.tgz", "../x")),
        FileErrorCode::InvalidPath
    );
    assert_eq!(
        code(extract(&root, "../a.tgz", "x")),
        FileErrorCode::InvalidPath
    );
    assert_eq!(
        code(extract(&root, "missing.zip", "x")),
        FileErrorCode::NotFound
    );
}

#[test]
fn the_files_of_an_account_are_packed_into_an_encrypted_stream_that_unpacks_the_same() {
    use builtpanel_backup::{KEY_LEN, StreamReader};
    use builtpanel_fileops::backup::pack_files;
    use std::io::Read as _;
    let s = Scratch::new("pack");
    let root = s.root();
    std::fs::create_dir_all(s.path("www/sub")).unwrap();
    std::fs::create_dir_all(s.path(".trash/x")).unwrap();
    std::fs::write(s.path(".trash/x/old"), "deleted").unwrap();
    std::fs::write(s.path("www/index.php"), "<?php echo 1;").unwrap();
    std::fs::write(
        s.path("www/sub/big.bin"),
        (0..400_000u32)
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    std::fs::write(s.path("empty"), "").unwrap();
    symlink("/etc/passwd", s.path("www/link")).unwrap();
    let key = [3u8; KEY_LEN];
    let mut stored = Vec::new();
    let packed = pack_files(&root, &key, None, &mut stored).unwrap();
    assert_eq!((packed.entries, packed.skipped), (5, 1), "{packed:?}");
    assert!(
        !stored.windows(5).any(|w| w == b"<?php"),
        "nothing readable is stored"
    );
    // Back: decrypt, decompress, list and read.
    let plain = StreamReader::new(&stored[..], &key);
    let mut archive = tar::Archive::new(zstd::stream::Decoder::new(plain).unwrap());
    let mut seen = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        seen.insert(name, body);
    }
    assert_eq!(
        seen.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "empty",
            "www/",
            "www/index.php",
            "www/sub/",
            "www/sub/big.bin"
        ]
    );
    assert_eq!(seen["www/index.php"], b"<?php echo 1;");
    assert_eq!(seen["www/sub/big.bin"].len(), 1_600_000);
    assert!(!seen.contains_key("www/link") && !seen.keys().any(|k| k.contains("trash")));
    // The wrong key, or a changed stream, does not open.
    let mut wrong = Vec::new();
    assert!(
        StreamReader::new(&stored[..], &[4u8; KEY_LEN])
            .read_to_end(&mut wrong)
            .is_err()
    );
}

#[test]
fn the_last_pass_of_a_transfer_packs_only_what_is_newer_than_the_first_copy() {
    use builtpanel_backup::{KEY_LEN, StreamReader};
    use builtpanel_fileops::backup::pack_files;
    use std::io::Read as _;
    let s = Scratch::new("since");
    let root = s.root();
    std::fs::create_dir_all(s.path("www/sub")).unwrap();
    std::fs::write(s.path("www/old.php"), "old").unwrap();
    std::fs::write(s.path("www/sub/old2.txt"), "old2").unwrap();
    // Wait out the clock's second, take the time, make the new ones.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(s.path("www/new.php"), "new").unwrap();
    std::fs::write(s.path("www/old.php"), "old, changed").unwrap();
    let key = [5u8; KEY_LEN];
    let mut stored = Vec::new();
    pack_files(&root, &key, Some(since), &mut stored).unwrap();
    let plain = StreamReader::new(&stored[..], &key);
    let mut archive = tar::Archive::new(zstd::stream::Decoder::new(plain).unwrap());
    let mut seen = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();
        seen.push((name, body));
    }
    // The folders are all there (the new files need their places); the files are the new
    // and the changed ones only.
    assert_eq!(
        seen,
        [
            ("www/".to_owned(), String::new()),
            ("www/new.php".to_owned(), "new".to_owned()),
            ("www/old.php".to_owned(), "old, changed".to_owned()),
            ("www/sub/".to_owned(), String::new()),
        ]
    );
}

#[test]
fn packed_files_are_put_back_over_changes_and_only_where_asked() {
    use builtpanel_backup::KEY_LEN;
    use builtpanel_fileops::backup::{pack_files, unpack_files};
    let s = Scratch::new("unpack");
    let root = s.root();
    std::fs::create_dir_all(s.path("www/sub")).unwrap();
    std::fs::create_dir_all(s.path("other")).unwrap();
    std::fs::write(s.path("www/index.php"), "original").unwrap();
    std::fs::write(s.path("www/sub/a.txt"), "a").unwrap();
    std::fs::write(s.path("other/b.txt"), "b").unwrap();
    let key = [5u8; KEY_LEN];
    let mut stored = Vec::new();
    pack_files(&root, &key, None, &mut stored).unwrap();
    // Damage: a changed file, a deleted file, a file made since, a link in the way.
    std::fs::write(s.path("www/index.php"), "hacked").unwrap();
    std::fs::remove_file(s.path("www/sub/a.txt")).unwrap();
    std::fs::remove_file(s.path("other/b.txt")).unwrap();
    symlink("/etc/passwd", s.path("other/b.txt")).unwrap();
    std::fs::write(s.path("www/new.txt"), "keep me").unwrap();
    // Only the folder asked for.
    let done = unpack_files(&root, &key, &stored[..], Some("www")).unwrap();
    assert_eq!(done.bytes, 9, "{done:?}");
    assert_eq!(
        std::fs::read_to_string(s.path("www/index.php")).unwrap(),
        "original"
    );
    assert_eq!(
        std::fs::read_to_string(s.path("www/sub/a.txt")).unwrap(),
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(s.path("www/new.txt")).unwrap(),
        "keep me"
    );
    assert!(
        std::fs::symlink_metadata(s.path("other/b.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // Everything: the link is replaced by the file, nothing is written through it.
    unpack_files(&root, &key, &stored[..], None).unwrap();
    assert_eq!(std::fs::read_to_string(s.path("other/b.txt")).unwrap(), "b");
    assert!(std::fs::read_to_string("/etc/passwd").unwrap().len() > 10);
    // A wrong key writes nothing.
    assert!(unpack_files(&root, &[6u8; KEY_LEN], &stored[..], None).is_err());
    // A cut stream is refused.
    assert!(unpack_files(&root, &key, &stored[..stored.len() - 20], None).is_err());
}
