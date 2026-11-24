//! SQL import and export for PostgreSQL for real (T07.12): a real server, a real `pg_dump`, a
//! small HTTP server standing in for the control plane. Ignored by default; needs the login
//! user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    dbroute::create_database,
    pg::{self, CONTAINER},
    sqlexport::export_sql,
    sqlimport::import_sql,
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbEngine, DbPassword, ErrorCode, Operation, Output};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const TOKEN: &str = "test-token-for-the-transfer";

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

/// One small server: `GET /dump` gives the current dump; `PUT /up/chunks/{n}` and `POST
/// /up/complete` take an export. All need the token.
struct Control {
    base: String,
    dump: Arc<Mutex<Vec<u8>>>,
    received: Arc<Mutex<Received>>,
    stop: Arc<AtomicBool>,
}

fn serve() -> Control {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (dump, received, stop) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Received::default())),
        Arc::new(AtomicBool::new(false)),
    );
    std::thread::spawn({
        let (dump, received, stop) = (dump.clone(), received.clone(), stop.clone());
        move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let (dump, received) = (dump.clone(), received.clone());
                std::thread::spawn(move || handle(&mut stream, &dump, &received));
            }
        }
    });
    Control {
        base,
        dump,
        received,
        stop,
    }
}

fn handle(stream: &mut TcpStream, dump: &Mutex<Vec<u8>>, received: &Mutex<Received>) {
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
    if !ok {
        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    if first.starts_with("GET /dump") {
        let data = dump.lock().unwrap().clone();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(&data);
        return;
    }
    let status = if let Some(n) = first
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

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn import_op(database: &str, control: &Control, data: &[u8], gzip: bool) -> Operation {
    *control.dump.lock().unwrap() = data.to_vec();
    Operation::ImportSql {
        username: "alice".into(),
        database: database.into(),
        import_id: Uuid::now_v7(),
        url: format!("{}/dump", control.base),
        size_bytes: data.len() as u64,
        sha256: sha(data),
        gzip,
        token: Some(DbPassword(TOKEN.into())),
        engine: DbEngine::Postgres,
        dedicated: None,
    }
}

/// Runs one statement as the server's superuser inside its container; the output.
fn sql(root: &std::path::Path, database: &str, statement: &str) -> String {
    let password = std::fs::read_to_string(root.join("secrets/postgres-root")).unwrap();
    let out = Command::new("docker")
        .args([
            "exec",
            "-e",
            &format!("PGPASSWORD={}", password.trim()),
            CONTAINER,
            "psql",
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            database,
            "-X",
            "-A",
            "-t",
            "-c",
            statement,
        ])
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_owned()
}

const DUMP: &str = "\\restrict TestKey123\n\
SET statement_timeout = 0;\n\
SELECT pg_catalog.set_config('search_path', '', false);\n\
CREATE TABLE public.items (id integer NOT NULL, name text, note text);\n\
CREATE SEQUENCE public.items_id_seq;\n\
ALTER TABLE public.items OWNER TO somebody_else;\n\
CREATE FUNCTION public.shout(t text) RETURNS text LANGUAGE plpgsql AS $$\n\
BEGIN\n  RETURN upper(t) || '; done';\nEND;\n$$;\n\
COPY public.items (id, name, note) FROM stdin;\n\
1\tone\tfirst; row\n\
2\ttwo\t\\N\n\
3\tthree\twith -- dashes\n\
\\.\n\
ALTER TABLE ONLY public.items ADD CONSTRAINT items_pkey PRIMARY KEY (id);\n\
GRANT SELECT ON TABLE public.items TO somebody_else;\n\
\\unrestrict TestKey123\n";

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_dump_is_run_as_a_limited_account_and_an_export_loads_again() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-pgx-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for volume in ["bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    let control = serve();
    let create = |name: &str| Operation::CreateDatabase {
        name: name.into(),
        engine: DbEngine::Postgres,
        dedicated: None,
    };
    create_database(context.clone(), create("alice_shop"))
        .await
        .unwrap();
    create_database(context.clone(), create("alice_copy"))
        .await
        .unwrap();

    // A dump with a COPY, a function body with semicolons, comments in the rows, an owner
    // that is not here and a grant: it loads, and what only concerns owners is let go.
    let done = import_sql(
        context.clone(),
        import_op("alice_shop", &control, DUMP.as_bytes(), false),
    )
    .await
    .unwrap();
    assert!(
        matches!(done, Output::SqlImported { statements } if statements >= 7),
        "{done:?}"
    );
    assert_eq!(
        sql(&root, "alice_shop", "select count(*) from public.items"),
        "3"
    );
    assert_eq!(
        sql(
            &root,
            "alice_shop",
            "select note from public.items where id = 1"
        ),
        "first; row"
    );
    assert_eq!(
        sql(
            &root,
            "alice_shop",
            "select note is null from public.items where id = 2"
        ),
        "t"
    );
    assert_eq!(
        sql(&root, "alice_shop", "select public.shout('x')"),
        "X; done"
    );
    // What the account made belongs to the database's `all` role; the account is gone.
    assert_eq!(
        sql(
            &root,
            "alice_shop",
            "select tableowner from pg_tables where tablename = 'items'"
        ),
        "alice_shop__all"
    );
    assert_eq!(
        sql(
            &root,
            "postgres",
            "select count(*) from pg_roles where rolname like '%\\_\\_imp'"
        ),
        "0"
    );
    // Compressed works the same.
    let done = import_sql(
        context.clone(),
        import_op("alice_copy", &control, &gz(DUMP.as_bytes()), true),
    )
    .await
    .unwrap();
    assert!(matches!(done, Output::SqlImported { .. }));
    assert_eq!(
        sql(&root, "alice_copy", "select count(*) from public.items"),
        "3"
    );

    // A psql command, a statement the server refuses, and a rights-grabbing one all stop it,
    // naming where.
    for (bad, what) in [
        ("\\connect postgres\nselect 1;\n", "psql command"),
        (
            "create table ok_before (a int);\ncreate table ok_before (a int);\n",
            "statement 2",
        ),
        ("create role evil superuser;\n", "statement 1"),
        ("copy public.items to program 'id';\n", "statement 1"),
        ("alter system set work_mem = '1GB';\n", "statement 1"),
        ("select pg_read_file('/etc/passwd');\n", "statement 1"),
    ] {
        let err = import_sql(
            context.clone(),
            import_op("alice_copy", &control, bad.as_bytes(), false),
        )
        .await
        .unwrap_err();
        assert!(!err.retryable, "{bad}");
        assert!(err.message.contains(what), "{bad}: {}", err.message);
    }
    // The account was removed each time, and the server's own state is untouched.
    assert_eq!(
        sql(
            &root,
            "postgres",
            "select count(*) from pg_roles where rolname like '%\\_\\_imp' or rolsuper and rolname <> 'postgres'"
        ),
        "0"
    );
    // The wrong checksum runs nothing.
    let mut wrong = import_op(
        "alice_copy",
        &control,
        b"create table nope (a int);\n",
        false,
    );
    if let Operation::ImportSql { sha256, .. } = &mut wrong {
        *sha256 = sha(b"something else");
    }
    assert!(import_sql(context.clone(), wrong).await.is_err());
    assert_eq!(
        sql(
            &root,
            "alice_copy",
            "select count(*) from pg_tables where tablename = 'nope'"
        ),
        "0"
    );
    // Bad arguments are refused before anything happens.
    let mut named = import_op("postgres", &control, b"select 1;", false);
    let err = import_sql(context.clone(), named.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    if let Operation::ImportSql { database, .. } = &mut named {
        *database = "a; drop database alice_shop".into();
    }
    assert_eq!(
        import_sql(context.clone(), named).await.unwrap_err().code,
        ErrorCode::InvalidArguments
    );

    // Export: the dump goes up in chunks as gzip, and is a dump the importer reads back.
    let export = |database: &str| Operation::ExportSql {
        username: "alice".into(),
        database: database.into(),
        export_id: Uuid::now_v7(),
        url: format!("{}/up", control.base),
        token: Some(DbPassword(TOKEN.into())),
        engine: DbEngine::Postgres,
        dedicated: None,
    };
    let sent = export_sql(context.clone(), export("alice_shop"))
        .await
        .unwrap();
    let Output::SqlExported { size_bytes } = sent else {
        panic!("{sent:?}");
    };
    let bytes: Vec<u8> = control
        .received
        .lock()
        .unwrap()
        .chunks
        .iter()
        .flatten()
        .copied()
        .collect();
    assert_eq!(bytes.len() as u64, size_bytes);
    let text = {
        let mut out = String::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut out)
            .unwrap();
        out
    };
    assert!(text.contains("COPY public.items"), "{text}");
    assert!(!text.contains("OWNER TO"), "an export carries no owners");
    assert!(
        control.received.lock().unwrap().complete.is_some(),
        "the control plane was told it is whole"
    );
    // The export account is gone.
    assert_eq!(
        sql(
            &root,
            "postgres",
            "select count(*) from pg_roles where rolname like '%\\_\\_exp'"
        ),
        "0"
    );
    create_database(context.clone(), create("alice_again"))
        .await
        .unwrap();
    import_sql(
        context.clone(),
        import_op("alice_again", &control, &bytes, true),
    )
    .await
    .unwrap();
    assert_eq!(
        sql(&root, "alice_again", "select count(*) from public.items"),
        "3"
    );
    assert_eq!(
        sql(&root, "alice_again", "select public.shout('y')"),
        "Y; done"
    );

    // Clean up.
    control.stop.store(true, Ordering::SeqCst);
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for volume in ["bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    let _ = Command::new("docker")
        .args(["network", "rm", "bp-pg-net"])
        .output();
    let _ = pg::port();
    let _ = std::fs::remove_dir_all(&root);
}
