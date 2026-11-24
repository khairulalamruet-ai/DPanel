//! The node's shared database server for real (T07.01): MariaDB in Docker, reached
//! from a tenant's own network and from nowhere else. Ignored by default; needs the
//! login user's rootless Docker and the image (the first run pulls it).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name, Network};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    db::{CONTAINER, allow_tenant_databases, ensure_db_server, port},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
use builtpanel_tenant_stack::network_name;

const BUSYBOX: &str =
    "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

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

/// Runs a shell snippet in a throw-away container on `network`; returns its exit code.
async fn probe(docker: &DockerManager, network: &Name, n: u32, script: &str) -> i64 {
    probe_with(docker, BUSYBOX, network, n, script, &[]).await
}

/// The same with a chosen image and environment.
async fn probe_with(
    docker: &DockerManager,
    image: &str,
    network: &Name,
    n: u32,
    script: &str,
    env: &[(&str, &str)],
) -> i64 {
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-dbprobe-{}-{n}", std::process::id())).unwrap(),
        ImageRef::parse(image).unwrap(),
    );
    spec.env = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    spec.command = vec!["sh".into(), "-c".into(), script.into()];
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
    docker.remove(&spec.name, true).await.unwrap();
    exit
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn the_database_server_is_reached_from_its_tenants_network_and_nowhere_else() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-db-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    let (a, b) = (
        TenantUsername::parse(&format!("da{}x", std::process::id())).unwrap(),
        TenantUsername::parse(&format!("db{}x", std::process::id())).unwrap(),
    );
    for t in [&a, &b] {
        docker
            .ensure_network(&network_name(t).unwrap(), false)
            .await
            .unwrap();
    }

    // The server starts, makes its password once, and is there for the next call.
    let ready = ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    let Output::DbServerReady { port: p, created } = ready else {
        panic!("{ready:?}");
    };
    assert_eq!(p, port());
    let password = std::fs::read_to_string(root.join("secrets/mariadb-root")).unwrap();
    assert_eq!(password.len(), 64);
    let again = ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    assert_eq!(
        again,
        Output::DbServerReady {
            port: p,
            created: false
        }
    );
    let _ = created;

    // Published on the node's loopback address and nowhere else.
    let published = Command::new("docker")
        .args(["port", CONTAINER, "3306/tcp"])
        .output()
        .unwrap();
    let published = String::from_utf8_lossy(&published.stdout).into_owned();
    assert!(
        published.lines().all(|l| l.starts_with("127.0.0.1:")),
        "{published:?}"
    );
    assert!(!published.trim().is_empty());

    // The tenant that was let in reaches it as `db`; the other tenant does not.
    let done = allow_tenant_databases(
        context.clone(),
        Operation::AllowTenantDatabases {
            username: a.as_str().to_owned(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        done,
        Output::TenantDbAllowed {
            network: network_name(&a).unwrap().as_str().to_owned()
        }
    );
    // (and doing it again is fine)
    allow_tenant_databases(
        context.clone(),
        Operation::AllowTenantDatabases {
            username: a.as_str().to_owned(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
        },
    )
    .await
    .unwrap();
    let reach = "nc -z -w 3 db 3306";
    assert_eq!(
        probe(&docker, &network_name(&a).unwrap(), 1, reach).await,
        0
    );
    assert_ne!(
        probe(&docker, &network_name(&b).unwrap(), 2, reach).await,
        0
    );

    // A tenant with no network here is refused.
    let err = allow_tenant_databases(
        context.clone(),
        Operation::AllowTenantDatabases {
            username: "nobodyhere".into(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);

    // Databases: made, found again, listed, injection refused, removed.
    {
        use builtpanel_node_agent::ops::db::{create_database, drop_database};
        use mysql_async::prelude::Queryable as _;
        let create = |name: &str| Operation::CreateDatabase {
            name: name.into(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        };
        assert_eq!(
            create_database(context.clone(), create("alice_shop"))
                .await
                .unwrap(),
            Output::DatabaseCreated { created: true }
        );
        assert_eq!(
            create_database(context.clone(), create("alice_shop"))
                .await
                .unwrap(),
            Output::DatabaseCreated { created: false }
        );
        let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(port())
            .user(Some("root"))
            .pass(Some(password.trim()))
            .into();
        let mut conn = mysql_async::Conn::new(opts).await.unwrap();
        let names = |mut conn: mysql_async::Conn| async move {
            let list: Vec<String> = conn.query("SHOW DATABASES").await.unwrap();
            (conn, list)
        };
        let (c, list) = names(conn).await;
        conn = c;
        assert!(list.contains(&"alice_shop".to_owned()), "{list:?}");
        let charset: Option<String> = conn
            .query_first("SELECT DEFAULT_CHARACTER_SET_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = 'alice_shop'")
            .await
            .unwrap();
        assert_eq!(charset.as_deref(), Some("utf8mb4"));
        // Nothing but a plain identifier reaches the server: these are refused first,
        // and the server's own databases are all still there.
        for bad in [
            "a; DROP DATABASE mysql",
            "x` ; DROP DATABASE alice_shop; --",
            "alice_shop' OR '1'='1",
            "mysql",
            "information_schema",
            "",
            "A",
        ] {
            for op in [
                Operation::CreateDatabase {
                    name: bad.into(),
                    engine: builtpanel_node_protocol::DbEngine::Mysql,
                    dedicated: None,
                },
                Operation::DropDatabase {
                    name: bad.into(),
                    engine: builtpanel_node_protocol::DbEngine::Mysql,
                    dedicated: None,
                },
            ] {
                let err = match op {
                    Operation::CreateDatabase { .. } => create_database(context.clone(), op).await,
                    _ => drop_database(context.clone(), op).await,
                }
                .unwrap_err();
                assert_eq!(err.code, ErrorCode::InvalidArguments, "{bad:?}");
                assert!(!err.retryable);
            }
        }
        let (c, list) = names(conn).await;
        conn = c;
        for kept in ["mysql", "information_schema", "alice_shop"] {
            assert!(list.contains(&kept.to_owned()), "{kept} is gone: {list:?}");
        }
        assert_eq!(
            drop_database(
                context.clone(),
                Operation::DropDatabase {
                    name: "alice_shop".into(),
                    engine: builtpanel_node_protocol::DbEngine::Mysql,
                    dedicated: None,
                }
            )
            .await
            .unwrap(),
            Output::DatabaseDropped { existed: true }
        );
        assert_eq!(
            drop_database(
                context.clone(),
                Operation::DropDatabase {
                    name: "alice_shop".into(),
                    engine: builtpanel_node_protocol::DbEngine::Mysql,
                    dedicated: None,
                }
            )
            .await
            .unwrap(),
            Output::DatabaseDropped { existed: false }
        );
        let (_, list) = names(conn).await;
        assert!(!list.contains(&"alice_shop".to_owned()));
    }

    // Users and grants, seen from inside the tenant's own network.
    {
        use builtpanel_node_agent::ops::db::{
            create_database, drop_database, drop_db_user, grant_db, revoke_db, upsert_db_user,
        };
        use mysql_async::prelude::Queryable as _;
        let image = builtpanel_tenant_stack::Allowlist::embedded()
            .unwrap()
            .current(builtpanel_tenant_stack::Role::Db)
            .unwrap()
            .as_str()
            .to_owned();
        for name in ["dbtest_shop", "dbtest_blog"] {
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
        let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(port())
            .user(Some("root"))
            .pass(Some(password.trim()))
            .into();
        let mut root = mysql_async::Conn::new(opts).await.unwrap();
        root.query_drop("CREATE TABLE dbtest_shop.t (a int)")
            .await
            .unwrap();
        root.query_drop("CREATE TABLE dbtest_blog.t (a int)")
            .await
            .unwrap();
        let (net_a, net_b) = (network_name(&a).unwrap(), network_name(&b).unwrap());
        // As the user, from a container on a network, with a statement; the exit code says if it worked.
        let sql = |n: u32, net: &Name, pass: &'static str, statement: &'static str| {
            let (docker, image) = (docker.clone(), image.clone());
            let net = net.clone();
            async move {
                let script = format!("mariadb -h db -u dbtest_web -N -e \"{statement}\"");
                probe_with(&docker, &image, &net, n, &script, &[("MYSQL_PWD", pass)]).await == 0
            }
        };
        let user = |password: Option<&str>| Operation::UpsertDbUser {
            username: a.as_str().to_owned(),
            name: "dbtest_web".into(),
            password: password.map(|p| builtpanel_node_protocol::DbPassword(p.into())),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        };
        let upsert = |op: Operation| upsert_db_user(context.clone(), op);
        // A new user needs a password, and a good one.
        for (bad, why) in [
            (None, "needed"),
            (Some("short"), "12"),
            (Some("alllowercaseonly"), "mixes"),
        ] {
            let err = upsert(user(bad)).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArguments);
            assert!(err.message.contains(why), "{}", err.message);
            assert!(!err.message.contains("alllowercase"));
        }
        let first = "First-Passw0rd!";
        let made = upsert(user(Some(first))).await.unwrap();
        let Output::DbUserSet { created, host } = made else {
            panic!("{made:?}")
        };
        assert!(created && host.contains('/'), "{host}");
        assert!(!host.contains('%'), "never from anywhere");
        // It is held for the tenant's network only.
        let hosts: Vec<String> = root
            .query("SELECT Host FROM mysql.user WHERE User = 'dbtest_web'")
            .await
            .unwrap();
        assert_eq!(hosts, std::slice::from_ref(&host));

        let grant = |access: &str, db: &str| Operation::GrantDb {
            database: db.into(),
            user: "dbtest_web".into(),
            access: access.into(),
            engine: builtpanel_node_protocol::DbEngine::Mysql,
            dedicated: None,
        };
        let g = |op: Operation| grant_db(context.clone(), op);
        g(grant("read_write", "dbtest_shop")).await.unwrap();
        // It sees its own database and nobody else's.
        let sees = |n: u32,
                    net: &Name,
                    pass: &'static str,
                    want: &'static str,
                    not: &'static str| {
            let (docker, image) = (docker.clone(), image.clone());
            let net = net.clone();
            async move {
                let script = format!(
                    "mariadb -h db -u dbtest_web -N -e 'SHOW DATABASES' | tee /dev/stderr | grep -qx {want} && ! mariadb -h db -u dbtest_web -N -e 'SHOW DATABASES' | grep -qxE '{not}'"
                );
                probe_with(&docker, &image, &net, n, &script, &[("MYSQL_PWD", pass)]).await == 0
            }
        };
        assert!(
            sees(
                11,
                &net_a,
                first,
                "dbtest_shop",
                "dbtest_blog|mysql|information_schema_x"
            )
            .await
        );
        assert!(
            sql(12, &net_a, first, "INSERT INTO dbtest_shop.t VALUES (1)").await,
            "read_write writes"
        );
        assert!(
            !sql(13, &net_a, first, "CREATE TABLE dbtest_shop.u (a int)").await,
            "but does not change the schema"
        );
        assert!(
            !sql(14, &net_a, first, "SELECT * FROM dbtest_blog.t").await,
            "and cannot read another database"
        );
        // From another tenant's network the server is not even there.
        assert!(!sql(15, &net_b, first, "SELECT 1").await);
        g(grant("read_only", "dbtest_shop")).await.unwrap();
        assert!(sql(16, &net_a, first, "SELECT * FROM dbtest_shop.t").await);
        assert!(
            !sql(17, &net_a, first, "INSERT INTO dbtest_shop.t VALUES (2)").await,
            "read_only cannot write"
        );
        g(grant("all", "dbtest_shop")).await.unwrap();
        assert!(
            sql(18, &net_a, first, "CREATE TABLE dbtest_shop.u (a int)").await,
            "all may"
        );
        assert!(
            !sql(19, &net_a, first, "CREATE DATABASE dbtest_extra").await,
            "but not beyond its database"
        );
        assert!(
            !sql(
                20,
                &net_a,
                first,
                "GRANT SELECT ON dbtest_shop.* TO 'x'@'%'"
            )
            .await,
            "and cannot pass it on"
        );
        // Refusals: unknown level, unknown user, injection in a name.
        for op in [
            grant("superuser", "dbtest_shop"),
            grant("all", "x; DROP DATABASE mysql"),
            Operation::GrantDb {
                database: "dbtest_shop".into(),
                user: "nobody_here".into(),
                access: "all".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
            Operation::GrantDb {
                database: "dbtest_shop".into(),
                user: "u'@'%' --".into(),
                access: "all".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        ] {
            assert_eq!(g(op).await.unwrap_err().code, ErrorCode::InvalidArguments);
        }
        // Revoked: no longer reads it.
        revoke_db(
            context.clone(),
            Operation::RevokeDb {
                database: "dbtest_shop".into(),
                user: "dbtest_web".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        revoke_db(
            context.clone(),
            Operation::RevokeDb {
                database: "dbtest_shop".into(),
                user: "dbtest_web".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        assert!(!sql(21, &net_a, first, "SELECT * FROM dbtest_shop.t").await);
        // A new password replaces the old one.
        g(grant("read_only", "dbtest_shop")).await.unwrap();
        let second = "Second-Passw0rd!";
        let again = upsert(user(Some(second))).await.unwrap();
        assert!(
            matches!(again, Output::DbUserSet { created: false, .. }),
            "{again:?}"
        );
        assert!(
            !sql(22, &net_a, first, "SELECT 1").await,
            "the old password is dead"
        );
        assert!(sql(23, &net_a, second, "SELECT * FROM dbtest_shop.t").await);
        // Remote access: two more networks stand in for two remote places. The user
        // may come from one of them (same password, same access) and from no other.
        {
            use builtpanel_node_agent::ops::db::set_db_remote_hosts;
            let (far1, far2) = (
                Name::parse(&format!("bp-dbfar1-{}", std::process::id())).unwrap(),
                Name::parse(&format!("bp-dbfar2-{}", std::process::id())).unwrap(),
            );
            for net in [&far1, &far2] {
                docker.ensure_network(net, false).await.unwrap();
                docker
                    .connect_network(&Name::parse(CONTAINER).unwrap(), net, "db")
                    .await
                    .unwrap();
            }
            let range = docker.network_subnet(&far1).await.unwrap();
            let remote = |cidrs: Vec<String>| Operation::SetDbRemoteHosts {
                username: a.as_str().to_owned(),
                name: "dbtest_web".into(),
                cidrs,
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            };
            assert!(
                !sql(30, &far1, second, "SELECT 1").await,
                "not before it is allowed"
            );
            let done = set_db_remote_hosts(context.clone(), remote(vec![range.clone()]))
                .await
                .unwrap();
            assert_eq!(done, Output::DbRemoteHostsSet { hosts: 1 });
            assert!(
                sql(31, &far1, second, "SELECT * FROM dbtest_shop.t").await,
                "from the added range, with its access"
            );
            assert!(
                !sql(32, &far1, second, "INSERT INTO dbtest_shop.t VALUES (9)").await,
                "read_only there too"
            );
            assert!(
                !sql(33, &far2, second, "SELECT 1").await,
                "from another range, no"
            );
            assert!(
                !sql(34, &far1, first, "SELECT 1").await,
                "and only with the password it has"
            );
            // Doing it again changes nothing; a grant given later reaches the added range too.
            set_db_remote_hosts(context.clone(), remote(vec![range.clone()]))
                .await
                .unwrap();
            g(grant("read_write", "dbtest_shop")).await.unwrap();
            assert!(sql(35, &far1, second, "INSERT INTO dbtest_shop.t VALUES (9)").await);
            // Nothing wider than a /8, nothing that is not an address.
            for bad in [
                "0.0.0.0/0",
                "%",
                "10.0.0.0/7",
                "203.0.113.7",
                "203.0.113.7/32'; --",
            ] {
                let err = set_db_remote_hosts(context.clone(), remote(vec![bad.into()]))
                    .await
                    .unwrap_err();
                assert_eq!(err.code, ErrorCode::InvalidArguments, "{bad}");
            }
            assert!(
                sql(36, &far1, second, "SELECT 1").await,
                "a refused change leaves it as it was"
            );
            // Taking it away ends the access.
            let done = set_db_remote_hosts(context.clone(), remote(vec![]))
                .await
                .unwrap();
            assert_eq!(done, Output::DbRemoteHostsSet { hosts: 0 });
            assert!(!sql(37, &far1, second, "SELECT 1").await);
            assert!(
                sql(38, &net_a, second, "SELECT 1").await,
                "its own network is untouched"
            );
            for net in [&far1, &far2] {
                let _ = docker.remove_network(net).await;
            }
        }
        // Sizes: the databases are listed (not the server's own), the one with a table has some size.
        let Output::DbSizes { items } = builtpanel_node_agent::ops::db::report_db_sizes(
            context.clone(),
            Operation::ReportDbSizes,
        )
        .await
        .unwrap() else {
            panic!("not sizes")
        };
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"dbtest_shop") && names.contains(&"dbtest_blog"),
            "{names:?}"
        );
        assert!(
            !names.contains(&"mysql") && !names.contains(&"information_schema"),
            "{names:?}"
        );
        assert!(
            items
                .iter()
                .find(|i| i.name == "dbtest_shop")
                .unwrap()
                .bytes
                > 0
        );
        // A database dropped and made again under the same name does not hand back
        // what was granted on the old one.
        revoke_db(
            context.clone(),
            Operation::RevokeDb {
                database: "dbtest_blog".into(),
                user: "dbtest_web".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        g(grant("read_only", "dbtest_blog")).await.unwrap();
        assert!(sql(40, &net_a, second, "SELECT * FROM dbtest_blog.t").await);
        drop_database(
            context.clone(),
            Operation::DropDatabase {
                name: "dbtest_blog".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        create_database(
            context.clone(),
            Operation::CreateDatabase {
                name: "dbtest_blog".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        root.query_drop("CREATE TABLE dbtest_blog.t (a int)")
            .await
            .unwrap();
        assert!(
            !sql(41, &net_a, second, "SELECT * FROM dbtest_blog.t").await,
            "the old grant is gone"
        );
        // Gone.
        let dropped = drop_db_user(
            context.clone(),
            Operation::DropDbUser {
                name: "dbtest_web".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(dropped, Output::DbUserDropped { existed: true });
        assert!(!sql(24, &net_a, second, "SELECT 1").await);
        let dropped = drop_db_user(
            context.clone(),
            Operation::DropDbUser {
                name: "dbtest_web".into(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(dropped, Output::DbUserDropped { existed: false });
        let _ = root.disconnect().await;
    }

    // Clean up (the data volume too: this is a test server).
    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    for t in [&a, &b] {
        let _ = docker.remove_network(&network_name(t).unwrap()).await;
    }
    let _ = docker
        .remove_network(&Name::parse("bp-db-net").unwrap())
        .await;
    let _ = std::fs::remove_dir_all(&root);
}
