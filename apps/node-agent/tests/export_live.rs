//! SQL export for real (T07.08): a real MariaDB, a real dump made by
//! `mariadb-dump` in its container, sent to a small HTTP server standing in for
//! the control plane, then imported into another database. Ignored by default;
//! needs the login user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    db::{CONTAINER, create_database, ensure_db_server, port},
    sqlexport::export_sql,
    sqlimport::import_sql,
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbPassword, ErrorCode, Operation, Output};
use mysql_async::prelude::Queryable as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const TOKEN: &str = "test-token-for-the-upload";

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

#[derive(Default)]
struct Received {
    chunks: Vec<Vec<u8>>,
    complete: Option<String>,
}

/// Takes `PUT /up/chunks/{n}` (in order) and `POST /up/complete`, with the token.
fn serve(received: Arc<Mutex<Received>>, stop: Arc<AtomicBool>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/up",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let received = received.clone();
            std::thread::spawn(move || handle(&mut stream, &received));
        }
    });
    url
}

fn handle(stream: &mut TcpStream, received: &Mutex<Received>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let first = text.lines().next().unwrap_or_default().to_owned();
    let length: usize = text
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    let ok = text
        .to_ascii_lowercase()
        .contains(&format!("authorization: bearer {TOKEN}"));
    let status = if !ok {
        "401 Unauthorized"
    } else if let Some(n) = first
        .strip_prefix("PUT /up/chunks/")
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse::<usize>().ok())
    {
        let mut r = received.lock().unwrap();
        if n == r.chunks.len() {
            r.chunks.push(body);
            "200 OK"
        } else {
            "409 Conflict"
        }
    } else if first.starts_with("POST /up/complete") {
        received.lock().unwrap().complete = Some(String::from_utf8(body).unwrap());
        "200 OK"
    } else {
        "404 Not Found"
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

fn op(database: &str, url: &str, token: Option<&str>, id: Uuid) -> Operation {
    Operation::ExportSql {
        username: "alice".into(),
        database: database.into(),
        export_id: id,
        url: url.into(),
        token: token.map(|t| DbPassword(t.into())),
        engine: builtpanel_node_protocol::DbEngine::Mysql,
        dedicated: None,
    }
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_database_is_dumped_compressed_and_sent_in_chunks_and_the_dump_loads_elsewhere() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    // Another live test may have left the server behind with its own root password.
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let root_dir = std::env::temp_dir().join(format!("bp-export-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root_dir);
    std::fs::create_dir_all(&root_dir).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker,
        vhosts_root: root_dir.join("vhosts"),
    });
    ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    for name in ["exptest_src", "exptest_copy"] {
        create_database(
            context.clone(),
            Operation::CreateDatabase {
                name: name.into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
    }
    let password = std::fs::read_to_string(root_dir.join("secrets/mariadb-root")).unwrap();
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port())
        .user(Some("root"))
        .pass(Some(password.trim()))
        .into();
    let mut admin = mysql_async::Conn::new(opts).await.unwrap();
    // Written without the database's name, as people write them (a dump keeps the
    // text of a trigger as it was typed).
    admin.query_drop("USE exptest_src").await.unwrap();
    for sql in [
        "CREATE TABLE items (id int PRIMARY KEY, note text, seen int DEFAULT 0, raw blob)",
        "CREATE TRIGGER items_bi BEFORE INSERT ON items FOR EACH ROW SET NEW.seen = NEW.id * 2",
        "CREATE VIEW v AS SELECT id, note FROM items",
        "CREATE PROCEDURE p() SELECT COUNT(*) FROM items",
        "INSERT INTO items (id, note, raw) VALUES (1, 'it''s; fine', 0x00ff10), (2, 'ünï©ode ☃', NULL)",
    ] {
        admin.query_drop(sql).await.unwrap();
    }
    // Enough rows that the compressed file is more than one chunk is too slow to
    // test; a few thousand random-looking rows are plenty for the code path.
    admin
        .query_drop("INSERT INTO items (id, note) SELECT seq + 10, MD5(seq) FROM seq_1_to_20000")
        .await
        .unwrap();

    let received = Arc::new(Mutex::new(Received::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let url = serve(received.clone(), stop.clone());
    let id = Uuid::now_v7();

    // Bad arguments never touch anything.
    for bad in [
        op("exptest_src", &url, None, id),
        op("exptest_src", "http://example.com/up", Some(TOKEN), id),
        op("exp test;", &url, Some(TOKEN), id),
    ] {
        let e = export_sql(context.clone(), bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    assert!(received.lock().unwrap().chunks.is_empty());
    // A wrong token is refused and the dump goes nowhere.
    let e = export_sql(context.clone(), op("exptest_src", &url, Some("wrong"), id))
        .await
        .unwrap_err();
    assert!(e.message.contains("refused"), "{e:?}");

    let done = export_sql(context.clone(), op("exptest_src", &url, Some(TOKEN), id))
        .await
        .unwrap();
    let Output::SqlExported { size_bytes } = done else {
        panic!("{done:?}")
    };
    let (file, complete): (Vec<u8>, String) = {
        let r = received.lock().unwrap();
        (r.chunks.concat(), r.complete.clone().unwrap())
    };
    assert_eq!(file.len() as u64, size_bytes);
    let said: serde_json::Value = serde_json::from_str(&complete).unwrap();
    assert_eq!(said["size_bytes"], size_bytes);
    assert_eq!(
        said["sha256"],
        Sha256::digest(&file)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    // Nothing is left behind on the node, not the file and not the account.
    let left: Vec<String> = std::fs::read_dir(root_dir.join("exports"))
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    let users: Vec<String> = admin
        .query("SELECT User FROM mysql.user WHERE User LIKE 'dhexport_%'")
        .await
        .unwrap();
    assert!(users.is_empty(), "{users:?}");

    // It is a gzip file of a dump that loads into another database.
    let mut text = String::new();
    flate2::read::GzDecoder::new(&file[..])
        .read_to_string(&mut text)
        .unwrap();
    assert!(
        text.contains("CREATE TABLE `items`"),
        "{}",
        &text[..text.len().min(400)]
    );
    assert!(
        !text.contains("USE `exptest_src`"),
        "no USE: it loads into any database"
    );
    assert!(text.contains("items_bi") && text.contains("CREATE VIEW") || text.contains("VIEW `v`"));
    // Served again to the import code.
    let data = Arc::new(Mutex::new(file.clone()));
    let _ = data;
    let copy_server = {
        // A tiny GET server for the import: reuse the same bytes.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/dump",
            listener.local_addr().unwrap().port()
        );
        let bytes = file.clone();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if s.read(&mut b).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(b[0]);
                }
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = s.write_all(&bytes);
            }
        });
        url
    };
    let loaded = import_sql(
        context.clone(),
        Operation::ImportSql {
            username: "alice".into(),
            database: "exptest_copy".into(),
            import_id: Uuid::now_v7(),
            url: copy_server,
            size_bytes: file.len() as u64,
            sha256: Sha256::digest(&file)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            gzip: true,
            token: Some(DbPassword("any".into())),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(loaded, Output::SqlImported { .. }), "{loaded:?}");
    let count: Option<i64> = admin
        .query_first("SELECT COUNT(*) FROM exptest_copy.items")
        .await
        .unwrap();
    assert_eq!(count, Some(20_002));
    let row: Option<(String, i32)> = admin
        .query_first("SELECT note, seen FROM exptest_copy.items WHERE id = 2")
        .await
        .unwrap();
    assert_eq!(row, Some(("ünï©ode ☃".into(), 4)));
    let procs: Option<i64> = admin
        .query_first("SELECT COUNT(*) FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = 'exptest_copy'")
        .await
        .unwrap();
    assert_eq!(procs, Some(1));
    stop.store(true, Ordering::SeqCst);
    let docker = context.docker.clone();
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let _ = std::fs::remove_dir_all(&root_dir);
}
