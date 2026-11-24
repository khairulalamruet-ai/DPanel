//! SQL import for real (T07.07): a real MariaDB, a real download from a small
//! HTTP server standing in for the control plane, real dumps. Ignored by default;
//! needs the login user's rootless Docker. BUILTPANEL_TEST_DUMP_MB sets the size of the
//! big dump (default 30).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    db::{CONTAINER, create_database, ensure_db_server, port},
    sqlimport::{import_sql, report_import_progress},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbPassword, ErrorCode, Operation, Output};
use mysql_async::prelude::Queryable as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const TOKEN: &str = "test-token-for-the-download";

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

/// A small server: GET /dump with the right bearer token returns the bytes (and
/// honours Range). With `cut_once` the first answer stops half way.
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
}

fn serve(data: Arc<Mutex<Vec<u8>>>, cut_once: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/dump",
        listener.local_addr().unwrap().port()
    );
    let (stop, requests) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let cut = Arc::new(AtomicBool::new(cut_once));
    std::thread::spawn({
        let (stop, requests) = (stop.clone(), requests.clone());
        move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let (data, cut, requests) = (data.clone(), cut.clone(), requests.clone());
                std::thread::spawn(move || {
                    handle(&mut stream, &data.lock().unwrap().clone(), &cut, &requests)
                });
            }
        }
    });
    Server {
        url,
        stop,
        requests,
    }
}

fn handle(stream: &mut TcpStream, data: &[u8], cut: &AtomicBool, requests: &AtomicUsize) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    requests.fetch_add(1, Ordering::SeqCst);
    let text = String::from_utf8_lossy(&head).into_owned();
    if !text.contains(&format!("Authorization: Bearer {TOKEN}"))
        && !text.contains(&format!("authorization: Bearer {TOKEN}"))
    {
        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let from = text
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("range: bytes=")
                .map(str::to_owned)
        })
        .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
    let body = &data[from.unwrap_or(0)..];
    let status = if from.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if cut.swap(false, Ordering::SeqCst) {
        let _ = stream.write_all(&body[..body.len() / 2]);
        return; // closes: the rest never comes
    }
    let _ = stream.write_all(body);
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn dump(table: &str) -> String {
    format!(
        "-- MariaDB dump 10.19\n\
         /*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;\n\
         /*!40101 SET NAMES utf8mb4 */;\n\
         DROP TABLE IF EXISTS `{table}`;\n\
         CREATE TABLE `{table}` (`id` int NOT NULL, `note` text, `seen` int DEFAULT 0, PRIMARY KEY (`id`)) ENGINE=InnoDB;\n\
         INSERT INTO `{table}` (id, note) VALUES (1,'semi;colon'),(2,'it''s'),(3,\"say \\\"hi\\\"\"),(4,'-- not a comment');\n\
         # a comment; with a semicolon\n\
         INSERT INTO `{table}` (id, note) VALUES (5,'/* nor this ; */');\n\
         DELIMITER ;;\n\
         CREATE TRIGGER `{table}_bi` BEFORE INSERT ON `{table}` FOR EACH ROW BEGIN\n  SET NEW.seen = NEW.id * 2;\nEND ;;\n\
         CREATE PROCEDURE `{table}_p`() BEGIN SELECT ';;' ; SELECT COUNT(*) FROM `{table}`; END ;;\n\
         DELIMITER ;\n\
         INSERT INTO `{table}` (id, note) VALUES (6,'after the trigger');\n"
    )
}

async fn root(password: &str) -> mysql_async::Conn {
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port())
        .user(Some("root"))
        .pass(Some(password))
        .into();
    mysql_async::Conn::new(opts).await.unwrap()
}

/// The arguments of an import, to change one at a time.
#[derive(Clone)]
struct Args {
    database: String,
    url: String,
    size: u64,
    sha: String,
    gzip: bool,
    token: Option<String>,
}

impl Args {
    fn new(database: &str, url: &str, data: &[u8]) -> Self {
        Self {
            database: database.into(),
            url: url.into(),
            size: data.len() as u64,
            sha: sha(data),
            gzip: false,
            token: Some(TOKEN.into()),
        }
    }
    fn gzip(mut self) -> Self {
        self.gzip = true;
        self
    }
    fn sha(mut self, sha: &str) -> Self {
        self.sha = sha.into();
        self
    }
    fn database(mut self, d: &str) -> Self {
        self.database = d.into();
        self
    }
    fn url(mut self, u: &str) -> Self {
        self.url = u.into();
        self
    }
    fn token(mut self, t: Option<&str>) -> Self {
        self.token = t.map(Into::into);
        self
    }
    fn size(mut self, s: u64) -> Self {
        self.size = s;
        self
    }
    fn op(&self, id: Uuid) -> Operation {
        Operation::ImportSql {
            username: "alice".into(),
            database: self.database.clone(),
            import_id: id,
            url: self.url.clone(),
            size_bytes: self.size,
            sha256: self.sha.clone(),
            gzip: self.gzip,
            token: self.token.clone().map(DbPassword),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        }
    }
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn dumps_are_downloaded_checked_and_run_with_the_rights_of_one_database() {
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
    let root_dir = std::env::temp_dir().join(format!("bp-import-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root_dir);
    std::fs::create_dir_all(&root_dir).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root_dir.join("vhosts"),
    });
    ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    for name in [
        "imptest_shop",
        "imptest_zip",
        "imptest_other",
        "imptest_big",
        "imptest_hostile",
    ] {
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
    let password = password.trim();
    let mut admin = root(password).await;
    admin
        .query_drop("CREATE TABLE imptest_other.keep (a int)")
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO imptest_other.keep VALUES (1)")
        .await
        .unwrap();

    // 1. A plain dump, with a trigger and a procedure written the way mysqldump does.
    let text = dump("items").into_bytes();
    let data = Arc::new(Mutex::new(text.clone()));
    let server = serve(data.clone(), false);
    let id = Uuid::now_v7();
    let done = import_sql(
        context.clone(),
        Args::new("imptest_shop", &server.url, &text).op(id),
    )
    .await
    .unwrap();
    let Output::SqlImported { statements } = done else {
        panic!("{done:?}")
    };
    assert!(statements >= 9, "{statements}");
    let rows: Vec<(i32, String, i32)> = admin
        .query("SELECT id, note, seen FROM imptest_shop.items ORDER BY id")
        .await
        .unwrap();
    assert_eq!(rows.len(), 6, "{rows:?}");
    assert_eq!(rows[0].1, "semi;colon");
    assert_eq!(rows[2].1, "say \"hi\"");
    assert_eq!(rows[3].1, "-- not a comment");
    assert_eq!(rows[4].1, "/* nor this ; */");
    assert_eq!(
        rows[5].2, 12,
        "the trigger ran for the row after it was made"
    );
    let triggers: Vec<String> = admin
        .query("SELECT TRIGGER_NAME FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = 'imptest_shop'")
        .await
        .unwrap();
    assert_eq!(triggers, ["items_bi"]);
    // The progress is there afterwards, and the file is gone.
    let progress = report_import_progress(
        context.clone(),
        Operation::ReportImportProgress { import_id: id },
    )
    .await
    .unwrap();
    let Output::ImportProgress {
        stage,
        bytes_done,
        bytes_total,
        statements: s,
        error,
    } = progress
    else {
        panic!()
    };
    assert_eq!(
        (stage.as_str(), bytes_done, bytes_total, error),
        ("done", text.len() as u64, text.len() as u64, None)
    );
    assert_eq!(s, statements);
    assert!(
        std::fs::read_dir(root_dir.join("imports"))
            .unwrap()
            .next()
            .is_none(),
        "the download is removed"
    );
    // Nobody is left behind.
    let leftovers: Vec<String> = admin
        .query("SELECT User FROM mysql.user WHERE User LIKE 'dhimport_%'")
        .await
        .unwrap();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // What the dump defined belongs to the database's own locked account, so it
    // keeps working after the import (and the dump's `DEFINER=` clauses are not needed).
    let definers: Vec<String> = admin
        .query(
            "SELECT DEFINER FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = 'imptest_shop'",
        )
        .await
        .unwrap();
    assert_eq!(definers, ["dhdef-imptest_shop@%"]);
    admin
        .query_drop("INSERT INTO imptest_shop.items (id, note) VALUES (7, 'later')")
        .await
        .unwrap();
    let later: Option<i32> = admin
        .query_first("SELECT seen FROM imptest_shop.items WHERE id = 7")
        .await
        .unwrap();
    assert_eq!(
        later,
        Some(14),
        "the trigger still works once the import is over"
    );
    let locked: Option<(String,)> = admin
        .query_first("SELECT JSON_VALUE(Priv, '$.account_locked') FROM mysql.global_priv WHERE User = 'dhdef-imptest_shop'")
        .await
        .unwrap();
    assert_eq!(locked, Some(("1".to_owned(),)), "nobody can log in as it");
    // Unknown imports are reported as unknown.
    let unknown = report_import_progress(
        context.clone(),
        Operation::ReportImportProgress {
            import_id: Uuid::now_v7(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(unknown, Output::ImportProgress { ref stage, .. } if stage == "unknown"));

    // 2. The same, compressed.
    let zipped = gz(&text);
    *data.lock().unwrap() = zipped.clone();
    import_sql(
        context.clone(),
        Args::new("imptest_zip", &server.url, &zipped)
            .gzip()
            .op(Uuid::now_v7()),
    )
    .await
    .unwrap();
    let n: i64 = admin
        .query_first("SELECT COUNT(*) FROM imptest_zip.items")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 6);
    // A file that is not gzip, said to be.
    *data.lock().unwrap() = text.clone();
    let err = import_sql(
        context.clone(),
        Args::new("imptest_zip", &server.url, &text)
            .gzip()
            .op(Uuid::now_v7()),
    )
    .await
    .unwrap_err();
    assert!(err.message.contains("gzip"), "{}", err.message);

    // 3. A dump that is not what was announced runs nothing.
    let err = import_sql(
        context.clone(),
        Args::new("imptest_other", &server.url, &text)
            .sha(&"0".repeat(64))
            .op(Uuid::now_v7()),
    )
    .await
    .unwrap_err();
    assert!(
        err.message.contains("SHA-256") && !err.retryable,
        "{}",
        err.message
    );
    let tables: Vec<String> = admin.query("SHOW TABLES FROM imptest_other").await.unwrap();
    assert_eq!(tables, ["keep"]);
    // A wrong credential is refused and says so.
    let bad = Args::new("imptest_other", &server.url, &text)
        .token(Some("wrong"))
        .op(Uuid::now_v7());
    let err = import_sql(context.clone(), bad).await.unwrap_err();
    assert!(
        err.message.contains("refused") && !err.retryable,
        "{}",
        err.message
    );
    // Bad arguments never reach the network.
    let before = server.requests.load(Ordering::SeqCst);
    let base = Args::new("imptest_shop", &server.url, &text);
    for bad in [
        base.clone()
            .database("x; DROP DATABASE mysql")
            .op(Uuid::now_v7()),
        base.clone().url("ftp://example.com/x").op(Uuid::now_v7()),
        base.clone().url("http://example.com/x").op(Uuid::now_v7()),
        base.clone().sha("XYZ").op(Uuid::now_v7()),
        base.clone().token(None).op(Uuid::now_v7()),
        base.clone().size(0).op(Uuid::now_v7()),
    ] {
        assert_eq!(
            import_sql(context.clone(), bad).await.unwrap_err().code,
            ErrorCode::InvalidArguments
        );
    }
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        before,
        "no request was made"
    );

    // 4. A dump that tries more than its own database stops at the first refusal.
    let hostile = "CREATE TABLE ok (a int);\nINSERT INTO ok VALUES (1);\nUSE mysql;\nDROP DATABASE imptest_other;\nCREATE DATABASE evil;\n"
        .as_bytes()
        .to_vec();
    *data.lock().unwrap() = hostile.clone();
    let err = import_sql(
        context.clone(),
        Args::new("imptest_hostile", &server.url, &hostile).op(Uuid::now_v7()),
    )
    .await
    .unwrap_err();
    assert!(
        err.message.starts_with("statement 3:") && !err.retryable,
        "{}",
        err.message
    );
    let n: i64 = admin
        .query_first("SELECT COUNT(*) FROM imptest_other.keep")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 1, "someone else's database is untouched");
    let dbs: Vec<String> = admin.query("SHOW DATABASES").await.unwrap();
    assert!(!dbs.contains(&"evil".to_owned()));
    for hostile in [
        "GRANT ALL PRIVILEGES ON *.* TO 'x'@'%' IDENTIFIED BY 'x';\n",
        "SET GLOBAL max_connections = 1;\n",
        "SELECT * FROM mysql.user;\n",
        "SELECT 1 INTO OUTFILE '/tmp/x';\n",
        "CREATE USER evil@'%';\n",
    ] {
        let bytes = hostile.as_bytes().to_vec();
        *data.lock().unwrap() = bytes.clone();
        let err = import_sql(
            context.clone(),
            Args::new("imptest_hostile", &server.url, &bytes).op(Uuid::now_v7()),
        )
        .await
        .unwrap_err();
        assert!(
            err.message.starts_with("statement 1:"),
            "{hostile}: {}",
            err.message
        );
    }
    let leftovers: Vec<String> = admin.query("SELECT User FROM mysql.user WHERE User LIKE 'dhimport_%' OR User = 'evil' OR User = 'x'").await.unwrap();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // 5. A broken download is taken up again where it stopped.
    *data.lock().unwrap() = text.clone();
    let cutting = serve(data.clone(), true);
    let id = Uuid::now_v7();
    let first = import_sql(
        context.clone(),
        Args::new("imptest_other", &cutting.url, &text).op(id),
    )
    .await
    .unwrap_err();
    assert!(first.retryable, "{}", first.message);
    let second = import_sql(
        context.clone(),
        Args::new("imptest_other", &cutting.url, &text).op(id),
    )
    .await;
    assert!(second.is_ok(), "{second:?}");
    assert_eq!(cutting.requests.load(Ordering::SeqCst), 2);
    cutting.stop.store(true, Ordering::SeqCst);

    // 6. A big one, compressed and not, with the progress moving while it runs.
    let mb: usize = std::env::var("BUILTPANEL_TEST_DUMP_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let mut big = String::from(
        "CREATE TABLE big (id bigint NOT NULL, a varchar(200), b varchar(200), PRIMARY KEY (id)) ENGINE=InnoDB;\n",
    );
    let mut id_n: u64 = 0;
    while big.len() < mb * 1024 * 1024 {
        big.push_str("INSERT INTO big VALUES ");
        for k in 0..4000 {
            id_n += 1;
            if k > 0 {
                big.push(',');
            }
            big.push_str(&format!(
                "({id_n},'row {id_n} with a ; and a quote '' inside','{:0>150}')",
                id_n
            ));
        }
        big.push_str(";\n");
    }
    let big = big.into_bytes();
    *data.lock().unwrap() = big.clone();
    let id = Uuid::now_v7();
    let watcher = tokio::spawn({
        let context = context.clone();
        async move {
            let mut seen = Vec::new();
            for _ in 0..100_000 {
                let p = report_import_progress(
                    context.clone(),
                    Operation::ReportImportProgress { import_id: id },
                )
                .await
                .unwrap();
                if let Output::ImportProgress {
                    stage, bytes_done, ..
                } = p
                {
                    if stage == "done" || stage == "failed" {
                        break;
                    }
                    if stage == "importing" {
                        seen.push(bytes_done);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            seen
        }
    });
    let started = Instant::now();
    import_sql(
        context.clone(),
        Args::new("imptest_big", &server.url, &big).op(id),
    )
    .await
    .unwrap();
    let took = started.elapsed();
    let seen = watcher.await.unwrap();
    let n: i64 = admin
        .query_first("SELECT COUNT(*) FROM imptest_big.big")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n as u64, id_n);
    eprintln!(
        "imported {} MiB ({n} rows) in {took:?}; progress seen {} times",
        big.len() >> 20,
        seen.len()
    );
    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "progress only moves forward"
    );
    assert!(
        seen.len() >= 2,
        "progress was visible while it ran: {seen:?}"
    );
    let zipped = gz(&big);
    *data.lock().unwrap() = zipped.clone();
    admin
        .query_drop("DROP TABLE imptest_big.big")
        .await
        .unwrap();
    import_sql(
        context.clone(),
        Args::new("imptest_big", &server.url, &zipped)
            .gzip()
            .op(Uuid::now_v7()),
    )
    .await
    .unwrap();
    let m: i64 = admin
        .query_first("SELECT COUNT(*) FROM imptest_big.big")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m as u64, id_n, "the compressed one gives the same");

    server.stop.store(true, Ordering::SeqCst);
    let _ = admin.disconnect().await;
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let _ = docker
        .remove_network(&Name::parse("bp-db-net").unwrap())
        .await;
    let _ = std::fs::remove_dir_all(&root_dir);
}
