//! `bpctl release`, end to end through the real binary (T17.14, T17.15): a key is made, a
//! release is signed, checked and installed; one changed byte stops it; the old files come back.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn bpctl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bpctl"))
        .args(args)
        .output()
        .unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn a_release_is_signed_checked_installed_refused_when_changed_and_rolled_back() {
    let base: PathBuf = std::env::temp_dir().join(format!("bp-release-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (down, live) = (base.join("down"), base.join("live"));
    std::fs::create_dir_all(&down).unwrap();
    std::fs::create_dir_all(&live).unwrap();
    let (key, keys, manifest) = (
        base.join("release.key"),
        base.join("keys.txt"),
        base.join("manifest.json"),
    );

    let made = bpctl(&["release", "keygen", "--kid", "release-1", "--out", s(&key)]);
    assert!(made.status.success(), "{made:?}");
    std::fs::write(&keys, &made.stdout).unwrap();
    // The private key is not given out twice.
    assert!(
        !bpctl(&["release", "keygen", "--kid", "release-1", "--out", s(&key)])
            .status
            .success()
    );

    std::fs::write(down.join("builtpanel-api"), b"api 2").unwrap();
    std::fs::write(down.join("builtpanel-worker"), b"worker 2").unwrap();
    std::fs::write(live.join("builtpanel-api"), b"api 1").unwrap();
    std::fs::write(live.join("builtpanel-worker"), b"worker 1").unwrap();
    let signed = bpctl(&[
        "release",
        "sign",
        "--key",
        s(&key),
        "--kid",
        "release-1",
        "--version",
        "2.0.0",
        "--commit",
        "abcdef0",
        "--out",
        s(&manifest),
        s(&down.join("builtpanel-api")),
        s(&down.join("builtpanel-worker")),
    ]);
    assert!(signed.status.success(), "{signed:?}");

    let ok = bpctl(&[
        "release",
        "verify",
        "--manifest",
        s(&manifest),
        "--keys",
        s(&keys),
        "--dir",
        s(&down),
    ]);
    assert!(ok.status.success(), "{ok:?}");

    // One changed byte: refused (data error), and nothing in the live folder moves.
    std::fs::write(down.join("builtpanel-worker"), b"worker 3").unwrap();
    let bad = bpctl(&[
        "release",
        "install",
        "--manifest",
        s(&manifest),
        "--keys",
        s(&keys),
        "--from",
        s(&down),
        "--to",
        s(&live),
    ]);
    assert_eq!(bad.status.code(), Some(65), "{bad:?}");
    assert_eq!(
        std::fs::read(live.join("builtpanel-api")).unwrap(),
        b"api 1"
    );
    assert_eq!(
        std::fs::read(live.join("builtpanel-worker")).unwrap(),
        b"worker 1"
    );

    // The real files install; the old ones come back with a rollback.
    std::fs::write(down.join("builtpanel-worker"), b"worker 2").unwrap();
    let done = bpctl(&[
        "release",
        "install",
        "--manifest",
        s(&manifest),
        "--keys",
        s(&keys),
        "--from",
        s(&down),
        "--to",
        s(&live),
    ]);
    assert!(done.status.success(), "{done:?}");
    assert_eq!(
        std::fs::read(live.join("builtpanel-api")).unwrap(),
        b"api 2"
    );
    let back = bpctl(&["release", "rollback", "--to", s(&live)]);
    assert!(back.status.success(), "{back:?}");
    assert_eq!(
        std::fs::read(live.join("builtpanel-api")).unwrap(),
        b"api 1"
    );
    assert_eq!(
        std::fs::read(live.join("builtpanel-worker")).unwrap(),
        b"worker 1"
    );

    // A manifest nobody trusts is refused.
    let stranger = base.join("none.txt");
    std::fs::write(&stranger, "").unwrap();
    let refused = bpctl(&[
        "release",
        "verify",
        "--manifest",
        s(&manifest),
        "--keys",
        s(&stranger),
        "--dir",
        s(&down),
    ]);
    assert_eq!(refused.status.code(), Some(65));
    let _ = std::fs::remove_dir_all(&base);
}
