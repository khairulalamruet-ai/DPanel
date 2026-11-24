//! The system backup against a real PostgreSQL, restored as the runbook says (T09.08).
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command};

use builtpanel_backup::{Manifest, StreamReader};
use builtpanel_backup_dest::{Destination as _, local::Local};
use builtpanel_system_backup::{Connection, Job, PART, derive_key, run_job, unpack};

fn psql(url: &str, sql: &str) -> String {
    let out = Command::new("psql")
        .args([url, "-Atqc", sql])
        .output()
        .expect("psql");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
#[ignore = "needs PostgreSQL (DATABASE_URL), pg_dump, pg_restore and psql"]
fn a_system_backup_is_stored_opened_and_restored_into_a_new_database() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipped: no DATABASE_URL");
        return;
    };
    let base = url.rsplit_once('/').unwrap().0.to_owned();
    let (source, fresh) = ("sysbk_source", "sysbk_fresh");
    for db in [source, fresh] {
        psql(&url, &format!("DROP DATABASE IF EXISTS {db}"));
    }
    psql(&url, &format!("CREATE DATABASE {source}"));
    let src = format!("{base}/{source}");
    psql(
        &src,
        "CREATE TABLE audit_logs (id serial PRIMARY KEY, action text)",
    );
    psql(
        &src,
        "INSERT INTO audit_logs (action) SELECT 'login ' || g FROM generate_series(1, 500) g",
    );
    psql(
        &src,
        "CREATE TABLE accounts (id serial PRIMARY KEY, name text); INSERT INTO accounts (name) VALUES ('alice'), ('bob')",
    );

    let dir = std::env::temp_dir().join(format!("bp-sysbk-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ca")).unwrap();
    std::fs::create_dir_all(dir.join("dest")).unwrap();
    std::fs::write(dir.join("control.toml"), "listen = \"127.0.0.1:8080\"\n").unwrap();
    std::fs::write(dir.join("ca/node_ca.crt"), "CERT").unwrap();
    std::fs::write(dir.join("ca/node_ca.key"), "CA-PRIVATE-KEY").unwrap();
    std::fs::write(dir.join("ca/master.key"), "THE-MASTER-KEY").unwrap();
    let master = "bWFzdGVyLWtleS1mb3ItdGhlLXRlc3Q=\n";
    let job = Job {
        id: "0123456789abcdef0123456789abcdef".into(),
        created_at: 1_700_000_000,
        kind: "local".into(),
        settings: serde_json::json!({"path": dir.join("dest").to_str().unwrap()}),
        secrets: serde_json::Value::Null,
        connection: Connection::parse(&src).unwrap(),
        config_file: Some(dir.join("control.toml")),
        secret_files: vec![],
        ca_dir: Some(dir.join("ca")),
        license_dir: None,
        scratch: dir.join("scratch"),
        key: derive_key(master),
        version: "0.1.0".into(),
    };
    let done = run_job(&job).expect("the backup");
    assert!(done.total_bytes > 0);
    assert!(
        std::fs::read_dir(dir.join("scratch"))
            .unwrap()
            .next()
            .is_none(),
        "the scratch folder is left clean"
    );

    // As an operator would: fetch the part and the manifest, open them with the master key.
    let dest = Local::open(&dir.join("dest")).unwrap();
    let manifest = Manifest::from_reader(StreamReader::new(
        dest.get(&format!("{}/manifest.json.enc", job.id)).unwrap(),
        &job.key,
    ))
    .unwrap();
    manifest
        .verify_part(PART, dest.get(&format!("{}/{PART}", job.id)).unwrap())
        .unwrap();
    // Nothing readable at the destination, and the master key is nowhere in it.
    let mut stored = Vec::new();
    std::io::Read::read_to_end(
        &mut dest.get(&format!("{}/{PART}", job.id)).unwrap(),
        &mut stored,
    )
    .unwrap();
    for needle in [&b"CA-PRIVATE-KEY"[..], b"THE-MASTER-KEY", b"alice"] {
        assert!(!stored.windows(needle.len()).any(|w| w == needle));
    }
    // The wrong master key opens nothing.
    assert!(
        unpack(
            &derive_key("another"),
            dest.get(&format!("{}/{PART}", job.id)).unwrap(),
            &dir.join("wrong")
        )
        .is_err()
    );
    let out: PathBuf = dir.join("opened");
    let names = unpack(
        &job.key,
        dest.get(&format!("{}/{PART}", job.id)).unwrap(),
        &out,
    )
    .unwrap();
    assert!(
        names.contains(&"database.dump".to_owned())
            && names.contains(&"pki/node_ca.key".to_owned())
    );
    assert!(!out.join("pki/master.key").exists());
    assert_eq!(
        std::fs::read_to_string(out.join("pki/node_ca.key")).unwrap(),
        "CA-PRIVATE-KEY"
    );

    // Restore into a new database, as the runbook says.
    psql(&url, &format!("CREATE DATABASE {fresh}"));
    let restored = Command::new("pg_restore")
        .args(["--no-owner", "--dbname", &format!("{base}/{fresh}")])
        .arg(out.join("database.dump"))
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let fresh_url = format!("{base}/{fresh}");
    assert_eq!(psql(&fresh_url, "SELECT count(*) FROM audit_logs"), "500");
    assert_eq!(
        psql(
            &fresh_url,
            "SELECT string_agg(name, ',' ORDER BY id) FROM accounts"
        ),
        "alice,bob"
    );

    // A destination that cannot be written leaves nothing behind.
    let mut bad = job.clone();
    bad.id = "fedcba9876543210fedcba9876543210".into();
    bad.settings = serde_json::json!({"path": "/proc"});
    assert!(run_job(&bad).is_err());
    assert!(
        std::fs::read_dir(dir.join("scratch"))
            .unwrap()
            .next()
            .is_none()
    );

    for db in [source, fresh] {
        psql(&url, &format!("DROP DATABASE IF EXISTS {db}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
