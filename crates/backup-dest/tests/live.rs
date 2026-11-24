//! The S3 and SFTP destinations against real servers (an S3 server (Scality CloudServer) and SFTPGo in containers).
//! Ignored by default; needs the login user's Docker, free ports 18000 and 12022, and the
//! images (`minio/minio`, `drakkan/sftpgo`) — they are pulled when missing.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{io::Read, process::Command};

use builtpanel_backup_dest::{
    DestError, Destination,
    s3::{S3, S3Config},
    sftp::{Login, Sftp, SftpConfig},
};
use sha2::{Digest, Sha256};

fn docker(args: &[&str]) -> (bool, String) {
    let out = Command::new("docker").args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.0]);
    }
}

fn big(n: usize) -> Vec<u8> {
    // Not compressible, different every call.
    let mut x = 0x9E37_79B9_7F4A_7C15_u64 ^ (n as u64);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What every destination must do, whatever its kind.
fn exercise(d: &dyn Destination, label: &str) {
    d.test().unwrap_or_else(|e| panic!("{label}: test: {e}"));
    // A small object (one request), and one of several multipart parts.
    let small = big(1000);
    assert_eq!(
        d.put("b1/small.enc", &mut &small[..]).unwrap(),
        1000,
        "{label}"
    );
    let large = big(40 * 1024 * 1024 + 123);
    assert_eq!(
        d.put("b1/large.enc", &mut &large[..]).unwrap(),
        large.len() as u64,
        "{label}"
    );
    d.put("b2/other.enc", &mut &small[..]).unwrap();
    let mut got = Vec::new();
    d.get("b1/large.enc")
        .unwrap()
        .read_to_end(&mut got)
        .unwrap();
    assert_eq!(
        sha(&got),
        sha(&large),
        "{label}: what was stored comes back the same"
    );
    assert_eq!(
        d.list("b1/").unwrap(),
        ["b1/large.enc", "b1/small.enc"],
        "{label}"
    );
    assert!(
        d.list("").unwrap().contains(&"b2/other.enc".to_owned()),
        "{label}"
    );
    assert!(
        !d.list("")
            .unwrap()
            .iter()
            .any(|n| n.ends_with(".part") || n.starts_with(".builtpanel-test")),
        "{label}: nothing half done or left over"
    );
    // Replacing.
    d.put("b1/small.enc", &mut &b"replaced"[..]).unwrap();
    let mut got = String::new();
    d.get("b1/small.enc")
        .unwrap()
        .read_to_string(&mut got)
        .unwrap();
    assert_eq!(got, "replaced", "{label}");
    // Removing (twice is fine), and what is not there.
    d.delete("b1/large.enc").unwrap();
    d.delete("b1/large.enc").unwrap();
    assert!(
        matches!(d.get("b1/large.enc"), Err(DestError::NotFound)),
        "{label}"
    );
    assert_eq!(
        d.put("../x", &mut &b"x"[..]),
        Err(DestError::BadName),
        "{label}"
    );
    // A stream that fails half way leaves nothing under the name.
    struct Breaks(usize);
    impl Read for Breaks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0 == 0 {
                return Err(std::io::Error::other("broken"));
            }
            let n = buf.len().min(self.0).min(1 << 20);
            buf[..n].fill(7);
            self.0 -= n;
            Ok(n)
        }
    }
    assert!(
        d.put("b3/broken.enc", &mut Breaks(20 * 1024 * 1024))
            .is_err(),
        "{label}"
    );
    assert!(
        d.list("b3/").unwrap().is_empty(),
        "{label}: a failed upload leaves nothing"
    );
}

#[test]
#[ignore = "needs Docker (see the file header)"]
fn an_s3_bucket_stores_big_objects_in_parts_and_refuses_wrong_keys() {
    let name = format!("bp-s3-test-{}", std::process::id());
    let _ = docker(&["rm", "-f", &name]);
    let (ok, out) = docker(&[
        "run",
        "-d",
        "--name",
        &name,
        "-p",
        "127.0.0.1:18000:8000",
        "-e",
        "S3BACKEND=mem",
        "-e",
        "REMOTE_MANAGEMENT_DISABLE=1",
        "-e",
        "SCALITY_ACCESS_KEY_ID=accessKey1",
        "-e",
        "SCALITY_SECRET_ACCESS_KEY=verySecretKey1",
        "zenko/cloudserver",
    ]);
    assert!(ok, "{out}");
    let _guard = Container(name);
    let config = S3Config {
        endpoint: "http://127.0.0.1:18000".into(),
        bucket: "dhbackups".into(),
        region: "us-east-1".into(),
        prefix: String::new(),
        path_style: true,
        access_key: "accessKey1".into(),
        secret_key: "verySecretKey1".into(),
    };
    // Wait for the server, make the bucket.
    let mut last = None;
    for _ in 0..90 {
        match S3::new(config.clone()).unwrap().create_bucket() {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert!(last.is_none(), "the S3 server did not come up: {last:?}");
    let s3 = S3::new(S3Config {
        prefix: "panel".into(),
        ..config.clone()
    })
    .unwrap();
    exercise(&s3, "s3");
    // Wrong secret, wrong bucket, nobody there.
    let bad = S3::new(S3Config {
        secret_key: "wrong-secret".into(),
        ..config.clone()
    })
    .unwrap();
    assert!(
        matches!(bad.test(), Err(DestError::Refused(_))),
        "{:?}",
        bad.test()
    );
    let nobucket = S3::new(S3Config {
        bucket: "nonexistent".into(),
        ..config.clone()
    })
    .unwrap();
    assert!(nobucket.test().is_err());
    let unreachable = S3::new(S3Config {
        endpoint: "http://127.0.0.1:1".into(),
        ..config
    })
    .unwrap();
    assert!(matches!(unreachable.test(), Err(DestError::Unreachable(_))));
}

#[test]
#[ignore = "needs Docker (see the file header)"]
fn an_sftp_folder_needs_the_pinned_key_and_stores_big_objects() {
    let name = format!("bp-sftp-dest-test-{}", std::process::id());
    let _ = docker(&["rm", "-f", &name]);
    let (ok, out) = docker(&[
        "run",
        "-d",
        "--name",
        &name,
        "-p",
        "127.0.0.1:12022:2022",
        "--user",
        "0",
        "drakkan/sftpgo",
        "sftpgo",
        "portable",
        "-u",
        "dhbackup",
        "-p",
        "backup-pass-1",
        "-d",
        "/tmp/backups",
        "-s",
        "2022",
        "-g",
        "*",
    ]);
    assert!(ok, "{out}");
    let _guard = Container(name);
    // The server needs a moment before it answers.
    std::thread::sleep(std::time::Duration::from_secs(6));
    let mut config = SftpConfig {
        host: "127.0.0.1".into(),
        port: 12022,
        user: "dhbackup".into(),
        path: "/".into(),
        host_key_sha256: None,
        login: Login::Password("backup-pass-1".into()),
    };
    // Without a pinned key nothing is trusted; the error says which key was shown.
    let mut shown = None;
    for _ in 0..60 {
        match Sftp::connect(&config) {
            Err(DestError::HostKey(fp)) => {
                shown = Some(fp);
                break;
            }
            other => {
                eprintln!("connect: {:?}", other.err());
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    let shown = shown.expect("the server showed no key");
    assert_eq!(shown.len(), 64);
    // A wrong pin is refused the same way.
    config.host_key_sha256 = Some("0".repeat(64));
    assert!(matches!(Sftp::connect(&config), Err(DestError::HostKey(fp)) if fp == shown));
    // The right one works.
    config.host_key_sha256 = Some(shown);
    let sftp = Sftp::connect(&config).unwrap();
    exercise(&sftp, "sftp");
    drop(sftp);
    // Wrong password.
    let bad = SftpConfig {
        login: Login::Password("nope".into()),
        ..config
    };
    assert!(matches!(Sftp::connect(&bad), Err(DestError::Refused(_))));
}
