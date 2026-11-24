//! The node's PostgreSQL server for real (T07.12): in Docker, reached from a tenant's own
//! network and from nowhere else; databases, users and access levels as the panel makes
//! them. Ignored by default; needs the login user's rootless Docker and the image.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name, Network};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    dbroute::{
        allow_tenant_databases, create_database, drop_database, drop_db_user, grant_db, revoke_db,
        set_db_remote_hosts, upsert_db_user,
    },
    pg::{self, CONTAINER},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbEngine, DbPassword, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::network_name;

const PG: DbEngine = DbEngine::Postgres;

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

/// Runs `psql` in a throw-away container on `network` as `user`; returns its exit code and
/// what it printed.
#[allow(clippy::too_many_arguments)]
async fn psql(
    docker: &DockerManager,
    image: &str,
    network: &Name,
    n: u32,
    user: &str,
    password: &str,
    database: &str,
    sql: &str,
) -> (i64, String) {
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-pgprobe-{}-{n}", std::process::id())).unwrap(),
        ImageRef::parse(image).unwrap(),
    );
    spec.env = [("PGPASSWORD".to_owned(), password.to_owned())]
        .into_iter()
        .collect();
    spec.command = vec![
        "psql".into(),
        "-h".into(),
        "pg".into(),
        "-U".into(),
        user.into(),
        "-d".into(),
        database.into(),
        "-X".into(),
        "-A".into(),
        "-t".into(),
        "-v".into(),
        "ON_ERROR_STOP=1".into(),
        "-c".into(),
        sql.into(),
    ];
    spec.network = Network::Named(network.clone());
    docker.pull_image(&spec.image).await.unwrap();
    docker.create_container(&spec).await.unwrap();
    docker.start(&spec.name).await.unwrap();
    let mut exit = -1;
    for _ in 0..300 {
        let info = docker.inspect(&spec.name).await.unwrap();
        if !info.running && info.status != "created" {
            exit = info.exit_code.unwrap_or(-1);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let logs = Command::new("docker")
        .args(["logs", spec.name.as_str()])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&logs.stdout),
        String::from_utf8_lossy(&logs.stderr)
    );
    docker.remove(&spec.name, true).await.unwrap();
    (exit, text.trim().to_owned())
}

fn create(name: &str) -> Operation {
    Operation::CreateDatabase {
        name: name.into(),
        engine: PG,
        dedicated: None,
    }
}

fn drop_op(name: &str) -> Operation {
    Operation::DropDatabase {
        name: name.into(),
        engine: PG,
        dedicated: None,
    }
}

fn user(tenant: &str, name: &str, password: Option<&str>) -> Operation {
    Operation::UpsertDbUser {
        username: tenant.into(),
        name: name.into(),
        password: password.map(|p| DbPassword(p.to_owned())),
        engine: PG,
        dedicated: None,
    }
}

fn grant(db: &str, user: &str, access: &str) -> Operation {
    Operation::GrantDb {
        database: db.into(),
        user: user.into(),
        access: access.into(),
        engine: PG,
        dedicated: None,
    }
}

fn revoke(db: &str, user: &str) -> Operation {
    Operation::RevokeDb {
        database: db.into(),
        user: user.into(),
        engine: PG,
        dedicated: None,
    }
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn postgresql_databases_users_and_access_levels_work_from_the_tenants_network_only() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-pg-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    // The server of an earlier run (its password is gone with its state directory).
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    for volume in ["bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    let pid = std::process::id();
    let (a, b) = (
        TenantUsername::parse(&format!("pa{pid}x")).unwrap(),
        TenantUsername::parse(&format!("pb{pid}x")).unwrap(),
    );
    for t in [&a, &b] {
        docker
            .ensure_network(&network_name(t).unwrap(), false)
            .await
            .unwrap();
    }
    let net_a = network_name(&a).unwrap();
    let net_b = network_name(&b).unwrap();
    let image = builtpanel_tenant_stack::Allowlist::embedded()
        .unwrap()
        .current(builtpanel_tenant_stack::Role::Pg)
        .unwrap()
        .as_str()
        .to_owned();

    // The server starts, makes its password once, and is there for the next call.
    let ready = pg::ensure_pg_server(context.clone(), Operation::EnsurePgServer)
        .await
        .unwrap();
    let Output::DbServerReady { port, .. } = ready else {
        panic!("{ready:?}");
    };
    assert_eq!(port, pg::port());
    assert_eq!(
        std::fs::read_to_string(root.join("secrets/postgres-root"))
            .unwrap()
            .len(),
        64
    );
    let published = Command::new("docker")
        .args(["port", CONTAINER, "5432/tcp"])
        .output()
        .unwrap();
    let published = String::from_utf8_lossy(&published.stdout).into_owned();
    assert!(
        published.lines().all(|l| l.starts_with("127.0.0.1:")),
        "{published:?}"
    );
    assert!(!published.trim().is_empty());

    // Only the tenant that was let in reaches it as `pg`.
    allow_tenant_databases(
        context.clone(),
        Operation::AllowTenantDatabases {
            username: a.as_str().to_owned(),
            engine: PG,
        },
    )
    .await
    .unwrap();
    let probe = |network: &Name, n: u32, user: &str, password: &str, db: &str, sql: &str| {
        let (docker, image, network, user, password, db, sql) = (
            docker.clone(),
            image.clone(),
            network.clone(),
            user.to_owned(),
            password.to_owned(),
            db.to_owned(),
            sql.to_owned(),
        );
        async move { psql(&docker, &image, &network, n, &user, &password, &db, &sql).await }
    };

    // A name that is not a plain identifier, or one the server keeps, is refused before
    // anything is sent.
    for bad in [
        "a; DROP DATABASE postgres",
        "x\" ; DROP DATABASE postgres; --",
        "postgres",
        "template0",
        "pg_catalog",
        "",
        "A",
        &"a".repeat(59),
    ] {
        for op in [create(bad), drop_op(bad)] {
            let err = match op {
                Operation::CreateDatabase { .. } => create_database(context.clone(), op).await,
                _ => drop_database(context.clone(), op).await,
            }
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArguments, "{bad:?}");
            assert!(!err.retryable);
        }
    }

    // Databases are made once, and found again.
    let (shop, blog) = (
        format!("{}_shop", a.as_str()),
        format!("{}_blog", a.as_str()),
    );
    assert_eq!(
        create_database(context.clone(), create(&shop))
            .await
            .unwrap(),
        Output::DatabaseCreated { created: true }
    );
    assert_eq!(
        create_database(context.clone(), create(&shop))
            .await
            .unwrap(),
        Output::DatabaseCreated { created: false }
    );
    create_database(context.clone(), create(&blog))
        .await
        .unwrap();

    // Users: a new one needs a password, a weak one is refused, the tenant must have a network.
    let (writer, reader, nobody) = (
        format!("{}_writer", a.as_str()),
        format!("{}_reader", a.as_str()),
        format!("{}_nobody", a.as_str()),
    );
    let (pw_w, pw_r, pw_n) = (
        "Writer-Pass-1234!",
        "Reader-Pass-1234!",
        "Nobody-Pass-1234!",
    );
    let err = upsert_db_user(context.clone(), user(a.as_str(), &writer, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    let err = upsert_db_user(context.clone(), user(a.as_str(), &writer, Some("short")))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    let err = upsert_db_user(context.clone(), user("nobodyhere", &writer, Some(pw_w)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    for (name, pw) in [(&writer, pw_w), (&reader, pw_r), (&nobody, pw_n)] {
        let done = upsert_db_user(context.clone(), user(a.as_str(), name, Some(pw)))
            .await
            .unwrap();
        assert!(
            matches!(done, Output::DbUserSet { created: true, .. }),
            "{done:?}"
        );
    }
    // The same user again is not made again.
    let again = upsert_db_user(context.clone(), user(a.as_str(), &reader, Some(pw_r)))
        .await
        .unwrap();
    assert!(matches!(again, Output::DbUserSet { created: false, .. }));

    // From the tenant's own network a user signs in; from another tenant's, nobody does.
    let (code, said) = probe(&net_a, 1, &writer, pw_w, "postgres", "select 1").await;
    assert_ne!(
        code, 0,
        "the server's own database is closed to tenants: {said}"
    );
    assert_ne!(
        probe(&net_b, 2, &writer, pw_w, &shop, "select 1").await.0,
        0,
        "another tenant's network cannot even find the server"
    );

    // Before any grant nobody can use the database.
    assert_ne!(
        probe(&net_a, 3, &writer, pw_w, &shop, "select 1").await.0,
        0
    );

    // Access levels.
    grant_db(context.clone(), grant(&shop, &writer, "read_write"))
        .await
        .unwrap();
    grant_db(context.clone(), grant(&shop, &reader, "read_only"))
        .await
        .unwrap();
    let (code, said) = probe(
        &net_a,
        4,
        &writer,
        pw_w,
        &shop,
        "create table items (id serial primary key, name text); insert into items (name) values ('one'), ('two')",
    )
    .await;
    assert_eq!(code, 0, "{said}");
    // The reader sees what the writer made (default privileges), and cannot change it.
    let (code, said) = probe(
        &net_a,
        5,
        &reader,
        pw_r,
        &shop,
        "select count(*) from items",
    )
    .await;
    assert_eq!((code, said.as_str()), (0, "2"), "{said}");
    let (code, _) = probe(
        &net_a,
        6,
        &reader,
        pw_r,
        &shop,
        "insert into items (name) values ('x')",
    )
    .await;
    assert_ne!(code, 0, "read only");
    let (code, _) = probe(&net_a, 7, &reader, pw_r, &shop, "create table t2 (a int)").await;
    assert_ne!(code, 0, "read only cannot make tables");
    // The writer changes data, and the sequence works for them.
    let (code, said) = probe(
        &net_a,
        8,
        &writer,
        pw_w,
        &shop,
        "insert into items (name) values ('three')",
    )
    .await;
    assert_eq!(code, 0, "{said}");
    // Nobody with no grant sees the database; nobody sees the other database.
    assert_ne!(
        probe(&net_a, 9, &nobody, pw_n, &shop, "select 1").await.0,
        0
    );
    assert_ne!(
        probe(&net_a, 10, &writer, pw_w, &blog, "select 1").await.0,
        0
    );
    // A table the reader's level cannot touch can not be reached through the catalogs either.
    let (code, _) = probe(&net_a, 11, &reader, pw_r, &shop, "drop table items").await;
    assert_ne!(code, 0);
    // A user cannot make a user or another database, or read the server's files.
    for sql in [
        "create role evil login superuser",
        "create database sneaky",
        "select pg_read_file('/etc/passwd')",
        "copy items to program 'id'",
        "alter system set work_mem = '1GB'",
    ] {
        assert_ne!(
            probe(&net_a, 12, &writer, pw_w, &shop, sql).await.0,
            0,
            "{sql}"
        );
    }

    // Changing the level replaces the old one.
    grant_db(context.clone(), grant(&shop, &reader, "read_write"))
        .await
        .unwrap();
    let (code, said) = probe(
        &net_a,
        13,
        &reader,
        pw_r,
        &shop,
        "insert into items (name) values ('by reader')",
    )
    .await;
    assert_eq!(code, 0, "{said}");
    grant_db(context.clone(), grant(&shop, &reader, "all"))
        .await
        .unwrap();
    let (code, said) = probe(
        &net_a,
        14,
        &reader,
        pw_r,
        &shop,
        "create table made_by_all (a int); insert into made_by_all values (1)",
    )
    .await;
    assert_eq!(code, 0, "{said}");
    // What a full-access user made is seen by the others at once.
    let (code, said) = probe(
        &net_a,
        15,
        &writer,
        pw_w,
        &shop,
        "select count(*) from made_by_all",
    )
    .await;
    assert_eq!((code, said.as_str()), (0, "1"), "{said}");

    // A new password is the one that works.
    upsert_db_user(
        context.clone(),
        user(a.as_str(), &reader, Some("Reader-New-Pass-77!")),
    )
    .await
    .unwrap();
    assert_ne!(
        probe(&net_a, 16, &reader, pw_r, &shop, "select 1").await.0,
        0
    );
    assert_eq!(
        probe(
            &net_a,
            17,
            &reader,
            "Reader-New-Pass-77!",
            &shop,
            "select 1"
        )
        .await
        .0,
        0
    );

    // Taking the access away shuts the door, and the data stays.
    revoke_db(context.clone(), revoke(&shop, &reader))
        .await
        .unwrap();
    assert_ne!(
        probe(
            &net_a,
            18,
            &reader,
            "Reader-New-Pass-77!",
            &shop,
            "select 1"
        )
        .await
        .0,
        0
    );
    let (code, said) = probe(
        &net_a,
        19,
        &writer,
        pw_w,
        &shop,
        "select count(*) from items",
    )
    .await;
    assert_eq!((code, said.as_str()), (0, "4"), "{said}");

    // Remote addresses: the file lets the user in from there too, and from nowhere else.
    set_db_remote_hosts(
        context.clone(),
        Operation::SetDbRemoteHosts {
            username: a.as_str().to_owned(),
            name: writer.clone(),
            cidrs: vec!["203.0.113.7/32".into()],
            engine: PG,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    let hba = std::fs::read_to_string(root.join("pg-conf/pg_hba.conf")).unwrap();
    assert!(
        hba.contains(&format!(
            "host all \"{writer}\" 203.0.113.7/32 scram-sha-256"
        )),
        "{hba}"
    );
    assert!(!hba.contains(&format!("\"{reader}\" 203.0.113.7")));
    set_db_remote_hosts(
        context.clone(),
        Operation::SetDbRemoteHosts {
            username: a.as_str().to_owned(),
            name: writer.clone(),
            cidrs: vec![],
            engine: PG,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    assert!(
        !std::fs::read_to_string(root.join("pg-conf/pg_hba.conf"))
            .unwrap()
            .contains("203.0.113.7")
    );
    // The server is still happy with the file (a user can still sign in).
    assert_eq!(
        probe(&net_a, 20, &writer, pw_w, &shop, "select 1").await.0,
        0
    );

    // Sizes are measured.
    let Output::DbSizes { items } = pg::report_sizes(context.clone(), Operation::ReportPgSizes)
        .await
        .unwrap()
    else {
        panic!("sizes")
    };
    assert!(
        items.iter().any(|d| d.name == shop && d.bytes > 0),
        "{items:?}"
    );
    assert!(!items.iter().any(|d| d.name == "postgres"));

    // Removing a user that made tables keeps the tables; removing a database removes it
    // and its roles, and doing it again is fine.
    let done = drop_db_user(
        context.clone(),
        Operation::DropDbUser {
            name: writer.clone(),
            engine: PG,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(done, Output::DbUserDropped { existed: true });
    assert_ne!(
        probe(&net_a, 21, &writer, pw_w, &shop, "select 1").await.0,
        0
    );
    assert_eq!(
        drop_database(context.clone(), drop_op(&shop))
            .await
            .unwrap(),
        Output::DatabaseDropped { existed: true }
    );
    assert_eq!(
        drop_database(context.clone(), drop_op(&shop))
            .await
            .unwrap(),
        Output::DatabaseDropped { existed: false }
    );
    // The group roles went with it: a database of the same name starts with nobody in it.
    create_database(context.clone(), create(&shop))
        .await
        .unwrap();
    assert_ne!(
        probe(
            &net_a,
            22,
            &reader,
            "Reader-New-Pass-77!",
            &shop,
            "select 1"
        )
        .await
        .0,
        0
    );

    // Clean up.
    for db in [&shop, &blog] {
        drop_database(context.clone(), drop_op(db)).await.unwrap();
    }
    for name in [&reader, &nobody] {
        drop_db_user(
            context.clone(),
            Operation::DropDbUser {
                name: name.clone(),
                engine: PG,
                dedicated: None,
            },
        )
        .await
        .unwrap();
    }
    for t in [&a, &b] {
        let _ = docker.remove_network(&network_name(t).unwrap()).await;
    }
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
    let _ = std::fs::remove_dir_all(&root);
}
