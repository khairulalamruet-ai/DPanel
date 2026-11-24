//! Fetching a backup over SSH, against a small SSH and SFTP server that runs in the test (T15.05).
//! It needs no database. Ignored by default only because it opens a local port.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use builtpanel_mod_migration::remote::{self, Credential, FetchError, Source};
use common::{PASSWORD, PATH, serve};
use secrecy::SecretString;

fn source(port: u16, password: &str, host_key: Option<&str>, path: &str, local: bool) -> Source {
    Source {
        host: "127.0.0.1".into(),
        port,
        user: "alice".into(),
        credential: Credential::Password(SecretString::from(password.to_owned())),
        host_key: host_key.map(str::to_owned),
        path: path.into(),
        allow_local: local,
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("bp-fetch-{}-{name}", uuid::Uuid::new_v4().simple()))
}

#[tokio::test]
#[ignore = "opens a local port"]
async fn a_backup_comes_over_when_the_host_key_and_login_are_right() {
    let content: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let (port, fingerprint) = serve(content.clone()).await;
    let dest = temp("ok");
    let mut reports = 0;
    let got = remote::fetch(
        source(port, PASSWORD, Some(&fingerprint), PATH, true),
        &dest,
        1 << 30,
        |_| {
            reports += 1;
        },
    )
    .await
    .unwrap();
    assert_eq!(got.bytes, content.len() as u64);
    assert_eq!(got.host_key, fingerprint);
    assert_eq!(std::fs::read(&dest).unwrap(), content);
    let _ = std::fs::remove_file(&dest);
}

#[tokio::test]
#[ignore = "opens a local port"]
async fn nothing_is_sent_without_a_known_host_key_and_a_wrong_key_is_refused() {
    let (port, fingerprint) = serve(b"data".to_vec()).await;
    let dest = temp("key");
    // No fingerprint: refused before the login, and the error names the one shown.
    let err = remote::fetch(
        source(port, PASSWORD, None, PATH, true),
        &dest,
        1 << 30,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(err, FetchError::UnknownHostKey(fingerprint.clone()));
    // Another fingerprint: refused.
    let err = remote::fetch(
        source(
            port,
            PASSWORD,
            Some("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            PATH,
            true,
        ),
        &dest,
        1 << 30,
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, FetchError::HostKeyChanged { ref got, .. } if *got == fingerprint),
        "{err:?}"
    );
    assert!(!dest.exists());
}

#[tokio::test]
#[ignore = "opens a local port"]
async fn a_wrong_password_a_missing_file_and_a_big_file_fail_cleanly() {
    let (port, fingerprint) = serve(vec![7u8; 100_000]).await;
    let dest = temp("fail");
    let err = remote::fetch(
        source(port, "wrong", Some(&fingerprint), PATH, true),
        &dest,
        1 << 30,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(err, FetchError::Login);
    assert!(
        !err.to_string().contains("wrong"),
        "the password is never in an error"
    );
    let err = remote::fetch(
        source(port, PASSWORD, Some(&fingerprint), "/nope", true),
        &dest,
        1 << 30,
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FetchError::Remote(_)), "{err:?}");
    let err = remote::fetch(
        source(port, PASSWORD, Some(&fingerprint), PATH, true),
        &dest,
        1000,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(err, FetchError::TooBig(100_000));
    assert!(!dest.exists(), "a failed fetch leaves no file");
}

#[tokio::test]
#[ignore = "opens a local port"]
async fn this_machine_is_refused_unless_a_test_allows_it() {
    let (port, fingerprint) = serve(b"data".to_vec()).await;
    let err = remote::fetch(
        source(port, PASSWORD, Some(&fingerprint), PATH, false),
        &temp("local"),
        1 << 30,
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FetchError::Refused(_)), "{err:?}");
}
