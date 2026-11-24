//! The restore drill (T19.07): a panel with real data is backed up, "lost", and brought back
//! on a new database exactly as `deploy/runbooks/system-restore.md` says - with the real
//! `bpctl` binary for every step it names (migrate, admin create, ca init, system-backup open).
//! Ignored by default; run with DATABASE_URL (superuser), pg_restore and psql on the PATH.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::Write as _,
    os::unix::fs::PermissionsExt as _,
    path::Path,
    process::{Command, Stdio},
};

use builtpanel_system_backup::{Connection, Job, PART, derive_key, run_job};

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

/// `bpctl ARGS` with the configuration of one panel database; returns what it printed.
fn bpctl(config: &Path, stdin: Option<&str>, args: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bpctl"))
        .args(args)
        .env("BUILTPANEL_CONFIG", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "bpctl {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// `table=rows` for every table of the panel, so two databases can be compared.
fn census(url: &str) -> String {
    let tables = psql(
        url,
        "SELECT string_agg(format('SELECT %L || ''='' || count(*) FROM %I', tablename, tablename), ' UNION ALL ' ORDER BY tablename)
         FROM pg_tables WHERE schemaname = 'public'",
    );
    psql(
        url,
        &format!("SELECT string_agg(r, ',' ORDER BY r) FROM ({tables}) t(r)"),
    )
}

#[test]
#[ignore = "needs PostgreSQL (DATABASE_URL), pg_restore and psql"]
fn a_lost_panel_comes_back_on_a_new_database_with_everything_in_it() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipped: no DATABASE_URL");
        return;
    };
    let base = url.rsplit_once('/').unwrap().0.to_owned();
    let (source, fresh) = ("drill_source", "drill_new");
    for db in [source, fresh] {
        psql(&url, &format!("DROP DATABASE IF EXISTS {db}"));
        psql(&url, &format!("CREATE DATABASE {db}"));
    }
    let (src_url, new_url) = (format!("{base}/{source}"), format!("{base}/{fresh}"));
    let dir = std::env::temp_dir().join(format!("bp-drill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("dest")).unwrap();
    // One configuration per server: the old one and the new one differ only in the database.
    let config = |name: &str, db_url: &str| {
        for (ext, text) in [("url", db_url), ("redis", "redis://127.0.0.1/")] {
            let file = dir.join(format!("{name}.{ext}"));
            std::fs::write(&file, text).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let path = dir.join(format!("{name}.toml"));
        std::fs::write(
            &path,
            format!(
                "listen = \"127.0.0.1:8080\"\n[database]\nurl_file = \"{0}/{1}.url\"\n[redis]\nurl_file = \"{0}/{1}.redis\"\n",
                dir.display(),
                name
            ),
        )
        .unwrap();
        path
    };
    let (old_config, new_config) = (config("old", &src_url), config("new", &new_url));

    // The panel as it was: migrated, an administrator, a CA, and some history.
    bpctl(&old_config, None, &["migrate"]);
    bpctl(
        &old_config,
        Some("correct-horse-battery-staple-91\n"),
        &[
            "admin",
            "create",
            "--username",
            "drilladmin",
            "--email",
            "drill@example.com",
            "--password-stdin",
        ],
    );
    bpctl(
        &old_config,
        None,
        &["ca", "init", "--dir", dir.join("ca").to_str().unwrap()],
    );
    let before = census(&src_url);
    let tables_with_rows = before.split(',').filter(|t| !t.ends_with("=0")).count();
    assert!(tables_with_rows >= 2, "something to restore: {before}");
    let hash_before = psql(
        &src_url,
        "SELECT password_hash FROM accounts WHERE username = 'drilladmin'",
    );
    assert!(hash_before.starts_with("$argon2"), "{hash_before}");

    // The nightly system backup.
    let master = "ZHJpbGwtbWFzdGVyLWtleS0wMDAwMDAwMDAwMDAwMDAw\n";
    std::fs::write(dir.join("master.key"), master).unwrap();
    let job = Job {
        id: "d41d8cd98f00b204e9800998ecf8427e".into(),
        created_at: 1_800_000_000,
        kind: "local".into(),
        settings: serde_json::json!({"path": dir.join("dest").to_str().unwrap()}),
        secrets: serde_json::Value::Null,
        connection: Connection::parse(&src_url).unwrap(),
        config_file: Some(old_config.clone()),
        secret_files: vec![("database_url".into(), dir.join("old.url"))],
        ca_dir: Some(dir.join("ca")),
        license_dir: None,
        scratch: dir.join("scratch"),
        key: derive_key(master),
        version: "drill".into(),
    };
    run_job(&job).expect("the system backup");

    // The server is lost: only the stored backup and the master key survive. Restore as the
    // runbook says.
    let opened = dir.join("restore");
    bpctl(
        &new_config,
        None,
        &[
            "system-backup",
            "open",
            "--master-key-file",
            dir.join("master.key").to_str().unwrap(),
            "--input",
            dir.join(format!("dest/{}/{PART}", job.id))
                .to_str()
                .unwrap(),
            "--output",
            opened.to_str().unwrap(),
        ],
    );
    let restored = Command::new("pg_restore")
        .args(["--no-owner", "--dbname", &new_url])
        .arg(opened.join("database.dump"))
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    for file in ["config/control.toml", "pki/node_ca.crt", "pki/node_ca.key"] {
        assert!(opened.join(file).exists(), "{file} is in the backup");
    }
    assert_eq!(
        std::fs::read(opened.join("pki/node_ca.crt")).unwrap(),
        std::fs::read(dir.join("ca/node_ca.crt")).unwrap(),
        "the same CA, so the nodes keep trusting the panel"
    );

    // The new panel: every table has the rows the old one had, the administrator's password
    // hash is the same, and the migrations tool finds nothing left to apply.
    assert_eq!(census(&new_url), before);
    assert_eq!(
        psql(
            &new_url,
            "SELECT password_hash FROM accounts WHERE username = 'drilladmin'"
        ),
        hash_before
    );
    let status = bpctl(&new_config, None, &["migrate", "info"]);
    assert!(!status.contains("pending"), "{status}");
    assert!(status.contains("applied"), "{status}");
    assert!(bpctl(&new_config, None, &["migrate"]).contains("migrations applied"));

    let _ = std::fs::remove_dir_all(&dir);
    for db in [source, fresh] {
        psql(&url, &format!("DROP DATABASE IF EXISTS {db}"));
    }
}
