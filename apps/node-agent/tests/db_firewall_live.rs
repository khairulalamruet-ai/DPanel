//! Remote access to the database server for real (T07.05b): the privileged helper
//! (as root) writes the packet filter, the agent opens the public port, and a
//! connection from this machine's own address gets through only while that address
//! is on the list. Ignored by default; needs the login user's rootless Docker, a
//! helper running as root (BUILTPANEL_TEST_HELPER_SOCKET) and the helper's
//! `--db-port` equal to the agent's public port (13306).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::Read as _,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{DockerManager, Name};
use builtpanel_node_agent::ops::{
    db::{CONTAINER, ensure_db_server, public_port, set_db_remote_access},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};

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

/// This machine's own outward address (what a connection to itself comes from).
fn own_address() -> String {
    let out = Command::new("ip")
        .args(["-4", "route", "get", "1.1.1.1"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut words = text.split_whitespace();
    while let Some(w) = words.next() {
        if w == "src" {
            return words.next().unwrap().to_owned();
        }
    }
    panic!("no address in {text:?}");
}

/// Whether the server's greeting arrives on `address:port` within a few seconds.
fn greeted(address: &str, port: u16) -> bool {
    let addr: SocketAddr = format!("{address}:{port}").parse().unwrap();
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) else {
        return false;
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut buf = [0u8; 16];
    // The first byte after the 4-byte header is the protocol version, 10.
    matches!(stream.read(&mut buf), Ok(n) if n > 5 && buf[4] == 10)
}

#[tokio::test]
#[ignore = "needs rootless Docker and a root helper (see the file header)"]
async fn the_public_port_lets_in_only_the_listed_addresses() {
    let (Some(socket), true) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        docker_socket().exists(),
    ) else {
        eprintln!("skipped: no helper socket or no Docker");
        return;
    };
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let root = std::env::temp_dir().join(format!("bp-dbfw-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: socket.into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.join("vhosts"),
    });
    let me = own_address();
    let port = public_port();
    let access = |cidrs: Vec<String>| Operation::SetDbRemoteAccess {
        cidrs,
        engine: builtpanel_node_protocol::DbEngine::Mysql,
    };

    ensure_db_server(context.clone(), Operation::EnsureDbServer)
        .await
        .unwrap();
    // Nothing allowed: the port is not even open.
    let done = set_db_remote_access(context.clone(), access(vec![]))
        .await
        .unwrap();
    assert_eq!(
        done,
        Output::DbRemoteAccessSet {
            ranges: 0,
            published: false
        }
    );
    assert!(!greeted(&me, port), "closed before anyone is allowed");

    // This machine's address allowed: the port is published and the server answers.
    let done = set_db_remote_access(context.clone(), access(vec![format!("{me}/32")]))
        .await
        .unwrap();
    assert_eq!(
        done,
        Output::DbRemoteAccessSet {
            ranges: 1,
            published: true
        }
    );
    if !greeted(&me, port) {
        // Say what the machine looks like, to make a failure easy to read.
        for (cmd, args) in [
            ("docker", vec!["port", CONTAINER]),
            (
                "docker",
                vec![
                    "inspect",
                    "--format",
                    "{{.Config.Labels}} {{.Created}}",
                    CONTAINER,
                ],
            ),
            ("ss", vec!["-ltn"]),
            ("nft", vec!["list", "ruleset"]),
        ] {
            let out = Command::new(cmd).args(&args).output();
            eprintln!(
                "{cmd} {args:?}:\n{}",
                out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default()
            );
        }
    }
    assert!(greeted(&me, port), "an allowed address reaches the server");
    // The agent's own loopback port still works, and the data survived the new container.
    assert!(greeted("127.0.0.1", builtpanel_node_agent::ops::db::port()));

    // Only other addresses allowed: this one is dropped (not refused: dropped).
    let done = set_db_remote_access(context.clone(), access(vec!["203.0.113.7/32".into()]))
        .await
        .unwrap();
    assert_eq!(
        done,
        Output::DbRemoteAccessSet {
            ranges: 1,
            published: true
        }
    );
    assert!(
        !greeted(&me, port),
        "an address that is not listed does not"
    );
    assert!(greeted("127.0.0.1", builtpanel_node_agent::ops::db::port()));

    // The list can grow and shrink; a range counts as all its addresses.
    let wide = format!("{}.0.0.0/8", me.split('.').next().unwrap());
    set_db_remote_access(
        context.clone(),
        access(vec![wide.clone(), "203.0.113.7/32".into()]),
    )
    .await
    .unwrap();
    assert!(greeted(&me, port), "inside an allowed range");
    set_db_remote_access(context.clone(), access(vec![]))
        .await
        .unwrap();
    assert!(!greeted(&me, port), "closed again for everybody");

    // Nothing wider than a /8, nothing that is not an address: refused before the
    // helper is asked, and the table stays as it was.
    for bad in ["0.0.0.0/0", "10.0.0.0/7", "x; flush ruleset", "203.0.113.7"] {
        let err = set_db_remote_access(context.clone(), access(vec![bad.into()]))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{bad}");
    }

    let _ = docker.remove(&Name::parse(CONTAINER).unwrap(), true).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-db-data").unwrap())
        .await;
    let _ = docker
        .remove_network(&Name::parse("bp-db-net").unwrap())
        .await;
    let _ = std::fs::remove_dir_all(&root);
}
