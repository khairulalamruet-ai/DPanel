//! SFTP and FTPS accounts for real (T08.11, T08.12): the real SFTPGo image, real
//! logins with curl. Ignored by default; needs the login user's rootless Docker and
//! `curl` with sftp. Ports 2222, 2121 and 21100-21119 on this machine must be free.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    ftp::{CONTAINER, drop_ftp_account, ftp_port, sftp_port, upsert_ftp_account},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbPassword, ErrorCode, Operation, Output};
use uuid::Uuid;

fn docker_socket() -> PathBuf {
    // The engine the live tests run against: another one (Kiri) than the rootless Docker of this user.
    if let Some(path) = std::env::var_os("BUILTPANEL_TEST_DOCKER_SOCKET") {
        return PathBuf::from(path);
    }
    let uid = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .unwrap();
    PathBuf::from(format!("/run/user/{uid}/docker.sock"))
}

#[allow(clippy::too_many_arguments)]
fn account(
    tenant: Uuid,
    uid: u32,
    name: &str,
    home: &str,
    password: Option<&str>,
    sftp: bool,
    ftps: bool,
    read_only: bool,
) -> Operation {
    Operation::UpsertFtpAccount {
        tenant_id: tenant,
        uid,
        name: name.into(),
        home: home.into(),
        password: password.map(|p| DbPassword(p.into())),
        sftp,
        ftps,
        ftp_plain: false,
        read_only,
        quota_mb: None,
        enabled: true,
    }
}

/// curl with a user, to an address; returns (exit code, stdout).
fn curl(args: &[&str]) -> (i32, String) {
    let out = Command::new("curl")
        .args(["-s", "-k", "--max-time", "20"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Runs a shell command in a root-in-the-namespace busybox over the tenants folder
/// (that is how the files get the owner a tenant's files have).
fn in_busybox(docker: &str, dir: &std::path::Path, script: &str) -> String {
    let out = Command::new(docker)
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/t", dir.display()),
            "busybox",
            "sh",
            "-c",
            script,
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
#[ignore = "needs rootless Docker, curl with sftp, free ports (see the file header)"]
async fn accounts_see_only_their_own_files_over_sftp_and_ftps() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    for name in [CONTAINER] {
        let _ = docker.remove(&Name::parse(name).unwrap(), true).await;
    }
    for v in ["bp-sftpgo-data", "bp-sftpgo-certs", "bp-sftpgo-tenants"] {
        let _ = docker.remove_volume(&Name::parse(v).unwrap()).await;
    }
    let base = std::env::temp_dir().join(format!("bp-ftp-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let tenants = base.join("tenants");
    std::fs::create_dir_all(&tenants).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: tenants.clone(),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Two tenants' files, owned by their own ids (as inside a rootless Docker).
    in_busybox(
        "docker",
        &tenants,
        &format!(
            "mkdir -p /t/{a}/www/sub /t/{b}/www && echo alpha > /t/{a}/www/a.txt && echo in-sub > /t/{a}/www/sub/s.txt \
             && echo bravo > /t/{b}/www/b.txt && ln -s /etc /t/{a}/www/evil && chown -R 10001:10001 /t/{a} && chown -R 10002:10002 /t/{b} \
             && chmod -R go-rwx /t/{a} /t/{b}"
        ),
    );

    // Bad arguments change nothing.
    for bad in [
        account(
            a,
            10_001,
            "Bad Name",
            "www",
            Some("password-1"),
            true,
            true,
            false,
        ),
        account(
            a,
            10_001,
            "alice_x",
            "../b",
            Some("password-1"),
            true,
            true,
            false,
        ),
        account(
            a,
            10_001,
            "alice_x",
            "www",
            Some("short"),
            true,
            true,
            false,
        ),
        account(
            a,
            5,
            "alice_x",
            "www",
            Some("password-1"),
            true,
            true,
            false,
        ),
    ] {
        let e = upsert_ftp_account(context.clone(), bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    assert!(
        docker
            .inspect(&Name::parse(CONTAINER).unwrap())
            .await
            .is_err(),
        "nothing was started for them"
    );

    let pw = "Correct-horse-1";
    for op in [
        account(a, 10_001, "alice_site", "www", Some(pw), true, true, false),
        account(a, 10_001, "alice_ro", "www", Some(pw), true, true, true),
        account(
            a,
            10_001,
            "alice_sub",
            "www/sub",
            Some(pw),
            true,
            false,
            false,
        ),
        account(b, 10_002, "bob_site", "www", Some(pw), true, true, false),
    ] {
        let done = upsert_ftp_account(context.clone(), op).await.unwrap();
        assert!(
            matches!(done, Output::FtpAccountSet { created: true }),
            "{done:?}"
        );
    }
    // A new account needs a password.
    let e = upsert_ftp_account(
        context.clone(),
        account(a, 10_001, "alice_new", "www", None, true, true, false),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    // Again: changed, not made; and a name that is someone else's is not taken over.
    let again = upsert_ftp_account(
        context.clone(),
        account(a, 10_001, "alice_site", "www", None, true, true, false),
    )
    .await
    .unwrap();
    assert!(
        matches!(again, Output::FtpAccountSet { created: false }),
        "{again:?}"
    );
    let e = upsert_ftp_account(
        context.clone(),
        account(b, 10_002, "alice_site", "www", Some(pw), true, true, false),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");

    let sftp =
        |user: &str, path: &str| format!("sftp://{user}:{pw}@127.0.0.1:{}/{path}", sftp_port());
    // Each sees its own files, and only those.
    let (code, listing) = curl(&["--list-only", &sftp("alice_site", "")]);
    assert_eq!(code, 0, "{listing}");
    assert!(
        listing.contains("a.txt") && listing.contains("sub"),
        "{listing}"
    );
    assert!(!listing.contains("b.txt"), "{listing}");
    let (_, listing) = curl(&["--list-only", &sftp("bob_site", "")]);
    assert!(
        listing.contains("b.txt") && !listing.contains("a.txt"),
        "{listing}"
    );
    // A smaller home is the whole world of the account.
    let (_, listing) = curl(&["--list-only", &sftp("alice_sub", "")]);
    assert!(
        listing.contains("s.txt") && !listing.contains("a.txt"),
        "{listing}"
    );
    let (code, got) = curl(&[&sftp("alice_sub", "../a.txt")]);
    assert!(code != 0 || !got.contains("alpha"), "went up: {code} {got}");
    // Reading works; a link out of the files does not lead out.
    let (code, got) = curl(&[&sftp("alice_site", "a.txt")]);
    assert_eq!((code, got.trim()), (0, "alpha"));
    let (code, got) = curl(&[&sftp("alice_site", "evil/passwd")]);
    assert!(
        code != 0 || got.is_empty(),
        "left the files through a link: {code} {got}"
    );
    let (code, got) = curl(&[&sftp("alice_site", "../b/www/b.txt")]);
    assert!(code != 0 || !got.contains("bravo"), "{code} {got}");
    // Wrong password.
    let (code, _) = curl(&[
        "--list-only",
        &format!("sftp://alice_site:nope@127.0.0.1:{}/", sftp_port()),
    ]);
    assert_ne!(code, 0);
    // Writing: allowed for the account, owned by the tenant; refused for the read-only one.
    let local = base.join("up.txt");
    std::fs::write(&local, "uploaded").unwrap();
    let local = local.to_str().unwrap();
    let (code, _) = curl(&["-T", local, &sftp("alice_site", "up.txt")]);
    assert_eq!(code, 0);
    let owner = in_busybox(
        "docker",
        &tenants,
        &format!("stat -c '%u:%g %a' /t/{a}/www/up.txt"),
    );
    assert!(
        owner.starts_with("10001:10001"),
        "the file is the tenant's own: {owner}"
    );
    let (code, _) = curl(&["-T", local, &sftp("alice_ro", "ro.txt")]);
    assert_ne!(code, 0, "read-only");
    assert!(!in_busybox("docker", &tenants, &format!("ls /t/{a}/www")).contains("ro.txt"));
    // The one without FTPS cannot use it; the others can, with TLS only.
    let ftps = |user: &str| format!("ftp://{user}:{pw}@127.0.0.1:{}/", ftp_port());
    let (code, listing) = curl(&["--ssl-reqd", "--list-only", &ftps("alice_site")]);
    assert_eq!(code, 0, "{listing}");
    assert!(
        listing.contains("a.txt") && !listing.contains("b.txt"),
        "{listing}"
    );
    let (code, _) = curl(&["--list-only", &ftps("alice_site")]);
    assert_ne!(code, 0, "plain FTP is refused");
    let (code, _) = curl(&["--ssl-reqd", "--list-only", &ftps("alice_sub")]);
    assert_ne!(code, 0, "an account without FTPS");

    // Removing an account ends its logins.
    let gone = drop_ftp_account(
        context.clone(),
        Operation::DropFtpAccount {
            name: "alice_site".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(gone, Output::FtpAccountDropped { existed: true }),
        "{gone:?}"
    );
    let (code, _) = curl(&["--list-only", &sftp("alice_site", "")]);
    assert_ne!(code, 0);
    let (_, listing) = curl(&["--list-only", &sftp("bob_site", "")]);
    assert!(
        listing.contains("b.txt"),
        "others are not affected: {listing}"
    );
    let none = drop_ftp_account(
        context.clone(),
        Operation::DropFtpAccount {
            name: "alice_site".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(none, Output::FtpAccountDropped { existed: false }),
        "{none:?}"
    );

    // Brute-force protection (T11.07): ten failed logins from one address ban that address
    // only. The loopback address and the machine's own public address are two different
    // clients to SFTPGo, provided the real source address reaches the container.
    let public = String::from_utf8(Command::new("hostname").arg("-I").output().unwrap().stdout)
        .unwrap()
        .split_whitespace()
        .find(|ip| ip.contains('.') && !ip.starts_with("127.") && !ip.starts_with("172."))
        .map(str::to_owned);
    if let Some(public) = public {
        let url =
            |host: &str, pass: &str| format!("sftp://bob_site:{pass}@{host}:{}/", sftp_port());
        let (code, _) = curl(&["--list-only", &url("127.0.0.1", pw)]);
        assert_eq!(code, 0, "a good login works before the failures");
        for _ in 0..12 {
            let (code, _) = curl(&["--list-only", &url("127.0.0.1", "wrong-password")]);
            assert_ne!(code, 0);
        }
        let (code, _) = curl(&["--list-only", &url("127.0.0.1", pw)]);
        assert_ne!(
            code, 0,
            "the address with ten failures is banned, even with the right password"
        );
        let (code, listing) = curl(&["--list-only", &url(&public, pw)]);
        assert_eq!(
            code, 0,
            "another address is not affected by that ban: {listing}"
        );
    } else {
        eprintln!("brute-force check skipped: no public address on this machine");
    }

    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    for v in ["bp-sftpgo-data", "bp-sftpgo-certs", "bp-sftpgo-tenants"] {
        let _ = docker.remove_volume(&Name::parse(v).unwrap()).await;
    }
    in_busybox("docker", &tenants, "rm -rf /t/* 2>/dev/null; true");
    let _ = std::fs::remove_dir_all(&base);
    let _ = ftp_port();
}
