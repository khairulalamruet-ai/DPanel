//! A tenant's own database server for real (T07.13), MariaDB and PostgreSQL: databases and
//! users move from the node's shared server to the tenant's own and back, with their data, their
//! passwords and their access, and the tenant's containers find whichever is the current one by
//! the same name. Ignored by default; needs the login user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name, Network};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    db, dbmigrate, dbroute,
    pg::{self},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{DbEngine, DbPassword, MigrateGrant, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, network_name};

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

/// Runs one statement as a user from a throw-away container on the tenant's network, by the
/// name the tenant's own containers use (`db` or `pg`): exit code and output.
#[allow(clippy::too_many_arguments)]
async fn sql_as(
    docker: &DockerManager,
    engine: DbEngine,
    network: &Name,
    n: u32,
    user: &str,
    password: &str,
    database: &str,
    statement: &str,
) -> (i64, String) {
    let role = match engine {
        DbEngine::Mysql => Role::Db,
        DbEngine::Postgres => Role::Pg,
    };
    let image = Allowlist::embedded()
        .unwrap()
        .current(role)
        .unwrap()
        .as_str()
        .to_owned();
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-dedprobe-{}-{n}", std::process::id())).unwrap(),
        ImageRef::parse(&image).unwrap(),
    );
    match engine {
        DbEngine::Mysql => {
            spec.entrypoint = vec!["mariadb".into()];
            spec.command = vec![
                "-h".into(),
                "db".into(),
                "-u".into(),
                user.into(),
                format!("-p{password}"),
                "-N".into(),
                "-B".into(),
                database.into(),
                "-e".into(),
                statement.into(),
            ];
        }
        DbEngine::Postgres => {
            spec.env = [("PGPASSWORD".to_owned(), password.to_owned())]
                .into_iter()
                .collect();
            spec.entrypoint = vec!["psql".into()];
            spec.command = vec![
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
                statement.into(),
            ];
        }
    }
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

fn running(name: &str) -> bool {
    let out = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim() == "true"
}

fn exists(name: &str) -> bool {
    Command::new("docker")
        .args(["inspect", name])
        .output()
        .unwrap()
        .status
        .success()
}

#[allow(clippy::too_many_lines)]
async fn round_trip(engine: DbEngine, context: &Arc<TenantContext>, docker: &DockerManager) {
    let pid = std::process::id();
    let tag = match engine {
        DbEngine::Mysql => "m",
        DbEngine::Postgres => "p",
    };
    let tenant = format!("d{tag}{pid}x");
    let user_name = TenantUsername::parse(&tenant).unwrap();
    let network = network_name(&user_name).unwrap();
    docker.ensure_network(&network, false).await.unwrap();
    let (database, user) = (format!("{tenant}_shop"), format!("{tenant}_web"));
    let password = "Dedicated-Pass-1234!";
    let (container, shared_container) = match engine {
        DbEngine::Mysql => (format!("bp-{tenant}-db"), db::CONTAINER.to_owned()),
        DbEngine::Postgres => (format!("bp-{tenant}-pg"), pg::CONTAINER.to_owned()),
    };

    // On the shared server: a database, a user with access, some data.
    dbroute::allow_tenant_databases(
        context.clone(),
        Operation::AllowTenantDatabases {
            username: tenant.clone(),
            engine,
        },
    )
    .await
    .unwrap();
    dbroute::create_database(
        context.clone(),
        Operation::CreateDatabase {
            name: database.clone(),
            engine,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    dbroute::upsert_db_user(
        context.clone(),
        Operation::UpsertDbUser {
            username: tenant.clone(),
            name: user.clone(),
            password: Some(DbPassword(password.into())),
            engine,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    dbroute::grant_db(
        context.clone(),
        Operation::GrantDb {
            database: database.clone(),
            user: user.clone(),
            access: "all".into(),
            engine,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    let run = |n: u32, statement: &'static str| {
        let (docker, network, user, db_name) = (
            docker.clone(),
            network.clone(),
            user.clone(),
            database.clone(),
        );
        async move {
            sql_as(
                &docker, engine, &network, n, &user, password, &db_name, statement,
            )
            .await
        }
    };
    let (code, said) = run(
        1,
        "create table items (id int primary key, name text); insert into items values (1, 'one'), (2, 'two')",
    )
    .await;
    assert_eq!(code, 0, "{said}");
    assert!(!exists(&container), "no server of its own yet");

    // To the tenant's own server.
    let migrate = |to_dedicated: bool| Operation::MigrateTenantDatabases {
        username: tenant.clone(),
        engine,
        to_dedicated,
        databases: vec![database.clone()],
        users: vec![user.clone()],
        grants: vec![MigrateGrant {
            database: database.clone(),
            user: user.clone(),
            access: "all".into(),
        }],
        remote_hosts: vec![],
    };
    let done = dbmigrate::migrate(context.clone(), migrate(true))
        .await
        .unwrap();
    assert_eq!(
        done,
        Output::DbMigrated {
            databases: 1,
            users: 1
        }
    );
    assert!(running(&container), "{container} runs");
    // The same name, the same password, the same data, now on the tenant's own server.
    let (code, said) = run(2, "select count(*) from items").await;
    assert_eq!((code, said.as_str()), (0, "2"), "{said}");
    let (code, said) = run(3, "insert into items values (3, 'three')").await;
    assert_eq!(code, 0, "{said}");
    // The shared server did not get it, and the tenant's network no longer leads to it.
    let (code, said) = run(4, "select count(*) from items").await;
    assert_eq!((code, said.as_str()), (0, "3"), "{said}");
    let shared_rows = match engine {
        DbEngine::Mysql => {
            let password =
                std::fs::read_to_string(context.vhosts_root.with_file_name("secrets/mariadb-root"))
                    .unwrap();
            Command::new("docker")
                .args([
                    "exec",
                    "-e",
                    &format!("MYSQL_PWD={}", password.trim()),
                    &shared_container,
                    "mariadb",
                    "-h",
                    "127.0.0.1",
                    "-u",
                    "root",
                    "-N",
                    "-B",
                    "-e",
                    &format!("select count(*) from `{database}`.items"),
                ])
                .output()
                .unwrap()
        }
        DbEngine::Postgres => {
            let password = std::fs::read_to_string(
                context.vhosts_root.with_file_name("secrets/postgres-root"),
            )
            .unwrap();
            Command::new("docker")
                .args([
                    "exec",
                    "-e",
                    &format!("PGPASSWORD={}", password.trim()),
                    &shared_container,
                    "psql",
                    "-h",
                    "127.0.0.1",
                    "-U",
                    "postgres",
                    "-d",
                    &database,
                    "-A",
                    "-t",
                    "-c",
                    "select count(*) from items",
                ])
                .output()
                .unwrap()
        }
    };
    assert_eq!(
        String::from_utf8_lossy(&shared_rows.stdout).trim(),
        "2",
        "the shared copy is as it was"
    );

    // Doing it again is fine: the copy is made new from the source.
    dbmigrate::migrate(context.clone(), migrate(true))
        .await
        .unwrap();
    let (code, said) = run(5, "select count(*) from items").await;
    assert_eq!((code, said.as_str()), (0, "2"), "{said}");

    // Back to the shared server, with what was done on the tenant's own.
    let (code, said) = run(6, "insert into items values (9, 'nine')").await;
    assert_eq!(code, 0, "{said}");
    dbmigrate::migrate(context.clone(), migrate(false))
        .await
        .unwrap();
    let (code, said) = run(7, "select count(*) from items").await;
    assert_eq!((code, said.as_str()), (0, "3"), "{said}");
    let removed = dbmigrate::remove(
        context.clone(),
        Operation::RemoveDedicatedDb {
            username: tenant.clone(),
            engine,
        },
    )
    .await
    .unwrap();
    assert_eq!(removed, Output::DedicatedDbRemoved { existed: true });
    assert!(!exists(&container));
    assert_eq!(
        dbmigrate::remove(
            context.clone(),
            Operation::RemoveDedicatedDb {
                username: tenant.clone(),
                engine,
            },
        )
        .await
        .unwrap(),
        Output::DedicatedDbRemoved { existed: false }
    );
    // Still there, from the tenant's side, on the shared server.
    let (code, said) = run(8, "select count(*) from items").await;
    assert_eq!((code, said.as_str()), (0, "3"), "{said}");

    // Clean up.
    dbroute::drop_database(
        context.clone(),
        Operation::DropDatabase {
            name: database.clone(),
            engine,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    dbroute::drop_db_user(
        context.clone(),
        Operation::DropDbUser {
            name: user.clone(),
            engine,
            dedicated: None,
        },
    )
    .await
    .unwrap();
    let _ = docker.remove_network(&network).await;
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_tenants_databases_move_to_its_own_server_and_back_with_data_and_passwords() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-ded-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    for container in [db::CONTAINER, pg::CONTAINER] {
        let _ = Command::new("docker")
            .args(["rm", "-f", container])
            .output();
    }
    for volume in ["bp-db-data", "bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    for engine in [DbEngine::Mysql, DbEngine::Postgres] {
        round_trip(engine, &context, &docker).await;
    }
    for container in [db::CONTAINER, pg::CONTAINER] {
        let _ = Command::new("docker")
            .args(["rm", "-f", container])
            .output();
    }
    for volume in ["bp-db-data", "bp-pg-data", "bp-pg-conf"] {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", volume])
            .output();
    }
    for net in ["bp-db-net", "bp-pg-net"] {
        let _ = Command::new("docker").args(["network", "rm", net]).output();
    }
    let _ = std::fs::remove_dir_all(&root);
}
