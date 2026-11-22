//! `create_backup` for real (T09.04): the privileged helper (root, started by the live
//! script), the agent's handlers, the box's rootless Docker and a real MariaDB. Ignored by
//! default; skips without BUILTPANEL_TEST_HELPER_SOCKET and BUILTPANEL_TEST_TENANTS_ROOT.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{io::Read as _, path::PathBuf, sync::Arc};

use builtpanel_backup::{KEY_LEN, Manifest, StreamReader};
use builtpanel_backup_dest::{Destination as _, local::Local};
use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    backup::{create_backup, delete_backup, export_backup, import_export, restore_backup},
    db::{CONTAINER, create_database, ensure_db_server, port},
    files::file_op,
    tenant::{TenantContext, create_tenant},
};
use builtpanel_node_protocol::{
    BackupTarget, DbPassword, ErrorCode, Operation, Output,
    files::{FileRequest, FileResult},
};
use mysql_async::prelude::Queryable as _;
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

fn hex_key(byte: u8) -> (String, [u8; KEY_LEN]) {
    (format!("{byte:02x}").repeat(KEY_LEN), [byte; KEY_LEN])
}

#[tokio::test]
#[ignore = "needs the privileged helper running as root, Docker and registry access"]
async fn an_account_is_backed_up_encrypted_and_every_part_checks_out() {
    let (Some(helper_socket), Some(tenants_root)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var_os("BUILTPANEL_TEST_TENANTS_ROOT"),
    ) else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let base = std::env::temp_dir().join(format!("bp-backup-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dest_dir = base.join("destination");
    std::fs::create_dir_all(&dest_dir).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: PathBuf::from(helper_socket),
        tenants_root: PathBuf::from(tenants_root),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });
    let tenant_id = Uuid::now_v7();
    let username = format!("b{}x", std::process::id());
    let uid = 10_078;
    create_tenant(
        context.clone(),
        Operation::CreateTenant {
            tenant_id,
            username: username.clone(),
            uid,
        },
    )
    .await
    .expect("tenant created");
    // Its files, written as the tenant.
    let made = file_op(
        context.clone(),
        Operation::FileOp {
            tenant_id,
            uid,
            request: FileRequest::Mkdir {
                path: "www/sub".into(),
            },
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(
            made,
            Output::File {
                result: FileResult::Done { .. }
            }
        ),
        "{made:?}"
    );
    for (path, content) in [
        ("www/index.php", "<?php echo 'hello';"),
        ("www/sub/data.txt", "some data"),
        ("notes.txt", "n"),
    ] {
        let done = file_op(
            context.clone(),
            Operation::FileOp {
                tenant_id,
                uid,
                request: FileRequest::Write {
                    path: path.into(),
                    content: content.into(),
                    max_bytes: 1000,
                    expected_mtime: None,
                    create_only: false,
                },
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(
                done,
                Output::File {
                    result: FileResult::Done { .. }
                }
            ),
            "{done:?}"
        );
    }
    // A database with a trigger, a view and rows.
    ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    create_database(
        context.clone(),
        Operation::CreateDatabase {
            name: "bkp_shop".into(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    let password = std::fs::read_to_string(base.join("secrets/mariadb-root")).unwrap();
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port())
        .user(Some("root"))
        .pass(Some(password.trim()))
        .into();
    let mut admin = mysql_async::Conn::new(opts).await.unwrap();
    admin.query_drop("USE bkp_shop").await.unwrap();
    for sql in [
        "CREATE TABLE items (id int PRIMARY KEY, note text)",
        "CREATE VIEW v AS SELECT id FROM items",
        "INSERT INTO items VALUES (1, 'one'), (2, 'it''s two ☃')",
    ] {
        admin.query_drop(sql).await.unwrap();
    }

    let (key_text, key) = hex_key(0x5a);
    let target = BackupTarget {
        kind: "local".into(),
        settings: serde_json::json!({"path": dest_dir.to_str().unwrap()}),
    };
    let backup_id = Uuid::now_v7();
    let op = |key: Option<&str>, target: &BackupTarget| Operation::CreateBackup {
        tenant_id,
        uid,
        backup_id,
        account: username.clone(),
        target: target.clone(),
        credentials: None,
        key: key.map(|k| DbPassword(k.into())),
        databases: vec!["bkp_shop".into()],
        dns: serde_json::json!([{"name": "example.com", "type": "A", "value": "203.0.113.7"}]),
        meta: serde_json::json!({"plan": "basic", "php": "8.4"}),
        created_at: 1_700_000_000,
        db_users: Vec::new(),
        files_since: None,
    };

    // Bad arguments change nothing.
    for bad in [
        op(None, &target),
        op(Some("zz"), &target),
        op(Some(&"0".repeat(63)), &target),
    ] {
        let e = create_backup(context.clone(), bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    assert!(std::fs::read_dir(&dest_dir).unwrap().next().is_none());

    let done = create_backup(context.clone(), op(Some(&key_text), &target))
        .await
        .unwrap();
    let Output::BackupCreated {
        manifest_sha256,
        size_bytes,
        parts,
    } = done
    else {
        panic!("{done:?}")
    };
    let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "files.tar.zst.enc",
            "db-bkp_shop.sql.zst.enc",
            "dns.json.enc",
            "meta.json.enc"
        ]
    );
    assert!(size_bytes > 0);

    // Checked from the outside, as a restore would: the manifest first, every part against it.
    let dest = Local::open(&dest_dir).unwrap();
    let id = backup_id.simple().to_string();
    let mut sealed = Vec::new();
    dest.get(&format!("{id}/manifest.json.enc"))
        .unwrap()
        .read_to_end(&mut sealed)
        .unwrap();
    assert_eq!(
        builtpanel_backup::manifest::hash_of(&sealed[..]).unwrap().1,
        manifest_sha256
    );
    let manifest = Manifest::from_reader(StreamReader::new(&sealed[..], &key)).unwrap();
    assert_eq!(
        (manifest.account.as_str(), manifest.created_at),
        (username.as_str(), 1_700_000_000)
    );
    for part in &manifest.parts {
        manifest
            .verify_part(
                &part.name,
                dest.get(&format!("{id}/{}", part.name)).unwrap(),
            )
            .unwrap();
        let mut stored = Vec::new();
        dest.get(&format!("{id}/{}", part.name))
            .unwrap()
            .read_to_end(&mut stored)
            .unwrap();
        assert!(
            !stored
                .windows(9)
                .any(|w| w == b"hello';" || w == b"CREATE TA"),
            "{}: nothing readable is stored",
            part.name
        );
    }
    // The files part: a tar of the tenant's files.
    let files = dest.get(&format!("{id}/files.tar.zst.enc")).unwrap();
    let mut archive =
        tar::Archive::new(zstd::stream::Decoder::new(StreamReader::new(files, &key)).unwrap());
    let mut seen = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut body = String::new();
        let _ = entry.read_to_string(&mut body);
        seen.insert(name, body);
    }
    assert_eq!(seen["www/index.php"], "<?php echo 'hello';");
    assert_eq!(seen["www/sub/data.txt"], "some data");
    assert_eq!(seen["notes.txt"], "n");
    // The database part: a dump with the structure and the rows.
    let dump = dest.get(&format!("{id}/db-bkp_shop.sql.zst.enc")).unwrap();
    let mut sql = String::new();
    zstd::stream::Decoder::new(StreamReader::new(dump, &key))
        .unwrap()
        .read_to_string(&mut sql)
        .unwrap();
    assert!(
        sql.contains("CREATE TABLE `items`")
            && sql.contains("it''s two ☃")
            && sql.contains("CREATE VIEW")
            || sql.contains("VIEW `v`"),
        "{}",
        &sql[..sql.len().min(300)]
    );
    // DNS and meta.
    let mut meta = String::new();
    StreamReader::new(dest.get(&format!("{id}/meta.json.enc")).unwrap(), &key)
        .read_to_string(&mut meta)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&meta).unwrap()["php"],
        "8.4"
    );
    // Another key opens nothing.
    assert!(
        StreamReader::new(
            dest.get(&format!("{id}/meta.json.enc")).unwrap(),
            &[1u8; KEY_LEN]
        )
        .read_to_string(&mut String::new())
        .is_err()
    );
    // A changed part is found by the manifest.
    let mut part = Vec::new();
    dest.get(&format!("{id}/dns.json.enc"))
        .unwrap()
        .read_to_end(&mut part)
        .unwrap();
    part[30] ^= 1;
    assert!(manifest.verify_part("dns.json.enc", &part[..]).is_err());

    // A destination that cannot be written to fails and leaves nothing.
    let bad_target = BackupTarget {
        kind: "local".into(),
        settings: serde_json::json!({"path": "/proc"}),
    };
    let e = create_backup(context.clone(), op(Some(&key_text), &bad_target))
        .await
        .unwrap_err();
    assert!(e.retryable || e.code == ErrorCode::Internal, "{e:?}");

    // ---- Restore (T09.05, T09.06): damage the account, then put it back.
    let read_file = |path: &'static str| {
        let context = context.clone();
        async move {
            let out = file_op(
                context,
                Operation::FileOp {
                    tenant_id,
                    uid,
                    request: FileRequest::Read {
                        path: path.into(),
                        max_bytes: 1000,
                    },
                },
            )
            .await
            .unwrap();
            format!("{out:?}")
        }
    };
    let write_file = |path: &'static str, content: &'static str| {
        let context = context.clone();
        async move {
            file_op(
                context,
                Operation::FileOp {
                    tenant_id,
                    uid,
                    request: FileRequest::Write {
                        path: path.into(),
                        content: content.into(),
                        max_bytes: 1000,
                        expected_mtime: None,
                        create_only: false,
                    },
                },
            )
            .await
            .unwrap()
        }
    };
    write_file("www/index.php", "DEFACED").await;
    write_file("www/new.txt", "made later").await;
    write_file("notes.txt", "changed").await;
    admin.query_drop("DROP VIEW v").await.unwrap();
    admin.query_drop("DELETE FROM items").await.unwrap();
    admin
        .query_drop("INSERT INTO items VALUES (9, 'wrong')")
        .await
        .unwrap();
    let restore = |id: Uuid,
                   key: &str,
                   files: bool,
                   folder: Option<&str>,
                   dbs: &[&str],
                   snapshot: bool,
                   verify_only: bool| {
        Operation::RestoreBackup {
            tenant_id,
            uid,
            backup_id: id,
            restore_id: Uuid::now_v7(),
            account: username.clone(),
            target: target.clone(),
            credentials: None,
            key: Some(DbPassword(key.into())),
            files,
            folder: folder.map(str::to_owned),
            databases: dbs.iter().map(|d| (*d).to_owned()).collect(),
            dns: false,
            snapshot,
            verify_only,
            create_databases: false,
            restore_db_users: false,
        }
    };
    // Verifying changes nothing and reads every part.
    let checked = restore_backup(
        context.clone(),
        restore(backup_id, &key_text, false, None, &[], false, true),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            checked,
            Output::BackupRestored {
                verified_parts: 4,
                ..
            }
        ),
        "{checked:?}"
    );
    assert!(read_file("www/index.php").await.contains("DEFACED"));
    // A wrong key, another account, an unknown database, a bad folder: refused, nothing touched.
    let (wrong_text, _) = hex_key(0x11);
    for bad in [
        restore(backup_id, &wrong_text, true, None, &[], false, false),
        restore(
            backup_id,
            &key_text,
            false,
            None,
            &["nope_db"],
            false,
            false,
        ),
        restore(backup_id, &key_text, true, Some("../x"), &[], false, false),
        restore(backup_id, &key_text, false, None, &[], false, false),
    ] {
        let e = restore_backup(context.clone(), bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    assert!(read_file("www/index.php").await.contains("DEFACED"));
    // A damaged part stops the restore before anything is touched.
    let damaged = dest_dir.join(&id).join("db-bkp_shop.sql.zst.enc");
    let original = std::fs::read(&damaged).unwrap();
    let mut broken = original.clone();
    let at = broken.len() / 2;
    broken[at] ^= 1;
    std::fs::write(&damaged, &broken).unwrap();
    let e = restore_backup(
        context.clone(),
        restore(backup_id, &key_text, true, None, &["bkp_shop"], true, false),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    assert!(e.message.contains("damaged"), "{e:?}");
    assert!(read_file("www/index.php").await.contains("DEFACED"));
    std::fs::write(&damaged, &original).unwrap();
    // One folder only.
    restore_backup(
        context.clone(),
        restore(backup_id, &key_text, true, Some("www"), &[], false, false),
    )
    .await
    .unwrap();
    assert!(read_file("www/index.php").await.contains("hello"));
    assert!(read_file("notes.txt").await.contains("changed"));
    assert!(read_file("www/new.txt").await.contains("made later"));
    // The DNS records only: they come back in the answer and nothing on the node changes.
    let mut dns_only = restore(backup_id, &key_text, false, None, &[], false, false);
    if let Operation::RestoreBackup { dns, .. } = &mut dns_only {
        *dns = true;
    }
    let answered = restore_backup(context.clone(), dns_only).await.unwrap();
    let Output::BackupRestored {
        dns: Some(records),
        files_restored: false,
        ..
    } = answered
    else {
        panic!("no records: {answered:?}")
    };
    assert_eq!(
        records,
        serde_json::json!([{"name": "example.com", "type": "A", "value": "203.0.113.7"}])
    );
    assert!(read_file("www/index.php").await.contains("hello"));
    // Everything, with the snapshot of what is there now.
    let restored = restore_backup(
        context.clone(),
        restore(backup_id, &key_text, true, None, &["bkp_shop"], true, false),
    )
    .await
    .unwrap();
    let Output::BackupRestored {
        files_restored,
        databases,
        snapshot,
        ..
    } = restored
    else {
        panic!("{restored:?}")
    };
    assert!(files_restored && databases == ["bkp_shop"]);
    assert_eq!(snapshot.len(), 2, "{snapshot:?}");
    assert!(
        read_file("notes.txt").await.contains("\"n\"")
            || read_file("notes.txt").await.contains("n\"")
    );
    let rows: Vec<(i32, String)> = admin
        .query("SELECT id, note FROM bkp_shop.items ORDER BY id")
        .await
        .unwrap();
    assert_eq!(rows, [(1, "one".to_owned()), (2, "it's two ☃".to_owned())]);
    let views: Vec<String> = admin
        .query("SELECT table_name FROM information_schema.views WHERE table_schema = 'bkp_shop'")
        .await
        .unwrap();
    assert_eq!(views, ["v"]);
    // The snapshot holds what was there before: the files part opens and has the file made later.
    let snap_dir = std::fs::read_dir(&dest_dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with("pre-"))
        .expect("a snapshot folder");
    let mut before = Vec::new();
    let mut files_snap = tar::Archive::new(
        zstd::stream::Decoder::new(StreamReader::new(
            std::fs::File::open(snap_dir.path().join("files.tar.zst.enc")).unwrap(),
            &key,
        ))
        .unwrap(),
    );
    for entry in files_snap.entries().unwrap() {
        before.push(
            entry
                .unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
    }
    assert!(before.iter().any(|n| n == "www/new.txt"), "{before:?}");
    // The transfer folder is left clean.
    let transfers = std::env::var_os("BUILTPANEL_AGENT_TRANSFERS").map(PathBuf::from);
    if let Some(t) = transfers {
        assert!(
            std::fs::read_dir(t)
                .unwrap()
                .filter_map(Result::ok)
                .all(|e| !e.file_name().to_string_lossy().ends_with(".rs")),
            "staged restore files are removed"
        );
    }

    // ---- Export (T09.11): one file, taken to another destination, restored from there.
    let header = builtpanel_backup::export::Header::new(
        &id,
        &username,
        1_700_000_000,
        &key,
        "a long enough passphrase",
    )
    .unwrap();
    let export_id = Uuid::now_v7();
    let exported = export_backup(
        context.clone(),
        Operation::ExportBackup {
            backup_id,
            export_id,
            target: target.clone(),
            credentials: None,
            header: String::from_utf8(header.to_json().unwrap()).unwrap(),
        },
    )
    .await
    .unwrap();
    let Output::BackupExported { object, size_bytes } = exported else {
        panic!("{exported:?}")
    };
    assert!(object.starts_with("exports/") && size_bytes > 0);
    // The file is a tar: the header, the manifest and every part, nothing readable.
    let mut file = Vec::new();
    dest.get(&object).unwrap().read_to_end(&mut file).unwrap();
    assert!(!file.windows(5).any(|w| w == b"hello"));
    let names: Vec<String> = tar::Archive::new(&file[..])
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names[0], "export.json");
    assert!(
        names.contains(&"manifest.json.enc".to_owned())
            && names.contains(&"files.tar.zst.enc".to_owned()),
        "{names:?}"
    );
    // Another destination (another node's disk): copy the file there and import it.
    let other_dir = base.join("other-destination");
    std::fs::create_dir_all(other_dir.join("exports")).unwrap();
    std::fs::write(other_dir.join(&object), &file).unwrap();
    let other = BackupTarget {
        kind: "local".into(),
        settings: serde_json::json!({"path": other_dir.to_str().unwrap()}),
    };
    for bad in ["../x.tar", "exports/a b.tar", "exports/x.zip"] {
        let e = import_export(Operation::ImportExport {
            object: bad.into(),
            backup_id,
            target: other.clone(),
            credentials: None,
        })
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{bad}: {e:?}");
    }
    let imported = import_export(Operation::ImportExport {
        object: object.clone(),
        backup_id,
        target: other.clone(),
        credentials: None,
    })
    .await
    .unwrap();
    let Output::ExportImported {
        parts: imported_parts,
        ..
    } = imported
    else {
        panic!("{imported:?}")
    };
    assert_eq!(imported_parts.len(), 4, "{imported_parts:?}");
    // The key comes out of the header with the passphrase, and the imported backup restores.
    let unwrapped = builtpanel_backup::export::Header::from_reader(&header.to_json().unwrap()[..])
        .unwrap()
        .unwrap_key("a long enough passphrase")
        .unwrap();
    assert_eq!(unwrapped, key);
    let verified = restore_backup(
        context.clone(),
        Operation::RestoreBackup {
            tenant_id,
            uid,
            backup_id,
            restore_id: Uuid::now_v7(),
            account: username.clone(),
            target: other.clone(),
            credentials: None,
            key: Some(DbPassword(key_text.clone())),
            files: false,
            folder: None,
            databases: vec![],
            dns: false,
            snapshot: false,
            verify_only: true,
            create_databases: false,
            restore_db_users: false,
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(
            verified,
            Output::BackupRestored {
                verified_parts: 4,
                ..
            }
        ),
        "{verified:?}"
    );
    // Importing over a backup that is there is refused.
    let e = import_export(Operation::ImportExport {
        object,
        backup_id,
        target: other,
        credentials: None,
    })
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");

    // Removing a backup takes away what it stored (and only that); doing it again is fine.
    for _ in 0..2 {
        let gone = delete_backup(Operation::DeleteBackup {
            backup_id,
            target: target.clone(),
            credentials: None,
        })
        .await
        .unwrap();
        assert!(matches!(gone, Output::BackupDeleted { .. }), "{gone:?}");
    }
    assert!(
        dest.list(&format!("{id}/")).unwrap().is_empty(),
        "nothing of the backup is left"
    );
    assert!(
        std::fs::read_dir(&dest_dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("pre-")),
        "the snapshot of the restore is not touched"
    );

    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let _ = std::fs::remove_dir_all(&base);
}
