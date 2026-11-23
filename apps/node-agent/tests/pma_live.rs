//! phpMyAdmin single sign-on for real (T07.09): a real MariaDB and the real
//! phpMyAdmin image. Ignored by default; needs the login user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    db::{CONTAINER, create_database, ensure_db_server, port},
    pma::{self, CONTAINER as PMA},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
use mysql_async::prelude::Queryable as _;
use sha2::{Digest as _, Sha256};
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

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn open_op(database: &str, token: &str, minutes: u32) -> Operation {
    Operation::OpenPhpMyAdmin {
        username: "alice".into(),
        database: database.into(),
        session_id: Uuid::now_v7(),
        token_hash: hash(token),
        session_minutes: minutes,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into()
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_link_signs_one_client_in_once_with_rights_on_one_database() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    for name in [CONTAINER, PMA] {
        let _ = docker.remove(&Name::parse(name).unwrap(), true).await;
    }
    for volume in ["bp-db-data", "bp-pma-sessions"] {
        let _ = docker.remove_volume(&Name::parse(volume).unwrap()).await;
    }
    let root_dir = std::env::temp_dir().join(format!("bp-pma-live-{}", std::process::id()));
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
    for name in ["pmatest_mine", "pmatest_theirs"] {
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
    admin
        .query_drop("CREATE TABLE pmatest_mine.t_mine (a int)")
        .await
        .unwrap();
    admin
        .query_drop("CREATE TABLE pmatest_theirs.t_theirs (a int)")
        .await
        .unwrap();

    // Bad arguments change nothing.
    for bad in [
        open_op("pmatest_mine", "x", 3),
        Operation::OpenPhpMyAdmin {
            username: "alice".into(),
            database: "pmatest_mine".into(),
            session_id: Uuid::now_v7(),
            token_hash: "nothex".into(),
            session_minutes: 60,
        },
        open_op("a b;", "x", 60),
    ] {
        let e = pma::open_phpmyadmin(context.clone(), bad)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    assert!(docker.inspect(&Name::parse(PMA).unwrap()).await.is_err());

    // The first sign-in starts phpMyAdmin.
    let token = format!("{}{}", Uuid::now_v7().simple(), Uuid::now_v7().simple());
    let opened = pma::open_phpmyadmin(context.clone(), open_op("pmatest_mine", &token, 30))
        .await
        .unwrap();
    assert!(
        matches!(opened, Output::PhpMyAdminOpened { .. }),
        "{opened:?}"
    );
    let base = format!("http://127.0.0.1:{}", pma::port());
    let http = agent();
    // Wait for Apache.
    let mut up = false;
    for _ in 0..60 {
        if http
            .get(&format!("{base}/index.php?dhsignon=ended"))
            .call()
            .is_ok_and(|r| r.status().as_u16() == 403)
        {
            up = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    assert!(up, "phpMyAdmin did not come up");

    // Not signed in: nothing is shown, and no other server can be named.
    let r = http.get(&format!("{base}/index.php")).call().unwrap();
    assert!(
        r.status().as_u16() == 302 || r.status().as_u16() == 403,
        "{}",
        r.status()
    );
    // A made-up token does nothing.
    let r = http
        .get(&format!("{base}/index.php?dhsignon={}", "0".repeat(64)))
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);
    // The real one signs in...
    let mut r = http
        .get(&format!("{base}/index.php?dhsignon={token}"))
        .call()
        .unwrap();
    assert_eq!(
        r.status().as_u16(),
        302,
        "{:?}",
        r.body_mut().read_to_string()
    );
    let cookie = r
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(cookie.contains("DHSIGNON="), "{cookie}");
    let location = r.headers()["location"].to_str().unwrap().to_owned();
    assert!(location.contains("pmatest_mine"), "{location}");
    // ... exactly once.
    let r = http
        .get(&format!("{base}/index.php?dhsignon={token}"))
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 403, "a link works once");

    // The signed-in session sees its own database and nothing else.
    let mut r = http
        .get(&format!("{base}/{location}"))
        .header("Cookie", &cookie)
        .call()
        .unwrap();
    let mut body = r.body_mut().read_to_string().unwrap();
    let mut hops = 0;
    // phpMyAdmin may redirect once more (its own cookies); follow a few.
    while (300..400).contains(&r.status().as_u16()) && hops < 4 {
        let next = r.headers()["location"].to_str().unwrap().to_owned();
        let url = if next.starts_with("http") {
            next
        } else {
            format!("{base}/{}", next.trim_start_matches('/'))
        };
        r = http.get(&url).header("Cookie", &cookie).call().unwrap();
        body = r.body_mut().read_to_string().unwrap();
        hops += 1;
    }
    assert_eq!(r.status().as_u16(), 200, "{body}");
    assert!(
        body.contains("t_mine") || body.contains("pmatest_mine"),
        "{body}"
    );
    assert!(
        !body.contains("pmatest_theirs"),
        "another database is visible"
    );
    assert!(!body.contains("t_theirs"));

    // The account can only touch its own database, even when used directly.
    let accounts: Vec<String> = admin
        .query("SELECT User FROM mysql.user WHERE User LIKE 'dhpma-%'")
        .await
        .unwrap();
    assert_eq!(accounts.len(), 1, "{accounts:?}");
    let grants: Vec<String> = admin
        .query(format!("SHOW GRANTS FOR '{}'@'%'", accounts[0]))
        .await
        .unwrap();
    let joined = grants.join("\n");
    assert!(joined.contains("`pmatest_mine`.*"), "{joined}");
    assert!(!joined.contains("pmatest_theirs"), "{joined}");
    assert!(!joined.contains("GRANT OPTION"), "{joined}");
    assert!(
        !joined.contains("ON *.* TO 'dhpma") || joined.contains("USAGE"),
        "{joined}"
    );

    // When the time is up the account goes (the sweep also removes unused links).
    std::fs::write(root_dir.join("pma/accounts").join(&accounts[0]), "1").unwrap();
    pma::sweep(&context).await;
    let left: Vec<String> = admin
        .query("SELECT User FROM mysql.user WHERE User LIKE 'dhpma-%'")
        .await
        .unwrap();
    assert!(left.is_empty(), "{left:?}");

    for name in [PMA, CONTAINER] {
        let _ = docker.remove(&Name::parse(name).unwrap(), true).await;
    }
    for volume in ["bp-db-data", "bp-pma-sessions"] {
        let _ = docker.remove_volume(&Name::parse(volume).unwrap()).await;
    }
    let _ = std::fs::remove_dir_all(&root_dir);
}
