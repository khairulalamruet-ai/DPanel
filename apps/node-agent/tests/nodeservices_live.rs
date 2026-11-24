//! The node's core services for real (T10.13): stand-in containers under other names than the
//! node's own, Docker's rootless daemon, the agent's handlers. Ignored by default; needs Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name};
use builtpanel_node_agent::ops::{
    nodeservices::{read_service_log_of, report_services_of, restart_node_service_of},
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

#[tokio::test]
#[ignore = "needs Docker"]
async fn core_services_are_reported_restarted_and_read_and_nothing_else_can_be_named() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let pid = std::process::id();
    let (db, pma, sftp) = (
        format!("bp-nsdb{pid}"),
        format!("bp-nspma{pid}"),
        format!("bp-nssftp{pid}"),
    );
    let containers = [
        ("database", db.as_str()),
        ("phpmyadmin", pma.as_str()),
        ("sftp", sftp.as_str()),
    ];
    // The database runs, phpMyAdmin has stopped, the file server was never installed.
    for (name, command) in [(&db, "echo database-started; sleep 600"), (&pma, "exit 3")] {
        let name = Name::parse(name).unwrap();
        let _ = docker.remove(&name, true).await;
        let mut spec = ContainerSpec::new(name.clone(), image.clone());
        spec.command = vec!["sh".into(), "-c".into(), command.into()];
        docker.create_container(&spec).await.unwrap();
        docker.start(&name).await.unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let base = std::env::temp_dir().join(format!("bp-nodeservices-live-{pid}"));
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.join("tenants"),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });

    let Output::Services { items } =
        report_services_of(context.clone(), Operation::ReportServices, &containers)
            .await
            .unwrap()
    else {
        panic!("not a report")
    };
    let state = |name: &str| {
        items
            .iter()
            .find(|i| i.name == name)
            .map(|i| i.state.as_str())
    };
    assert_eq!(state("agent"), Some("running"));
    assert_eq!(state("database"), Some("running"));
    assert_eq!(state("phpmyadmin"), Some("stopped"));
    assert_eq!(state("sftp"), Some("not_installed"));
    let stopped = items.iter().find(|i| i.name == "phpmyadmin").unwrap();
    assert!(stopped.detail.contains("exit code 3"), "{stopped:?}");

    // The log, and a restart that shows as a second start in it.
    let log = |tail: u32| {
        let context = context.clone();
        async move {
            read_service_log_of(
                context,
                Operation::ReadServiceLog {
                    name: "database".into(),
                    tail,
                },
                &containers,
            )
            .await
        }
    };
    let Output::LogLines { lines, .. } = log(50).await.unwrap() else {
        panic!("not a log")
    };
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.text == "database-started")
            .count(),
        1
    );
    let restarted = restart_node_service_of(
        context.clone(),
        Operation::RestartNodeService {
            name: "database".into(),
        },
        &containers,
    )
    .await
    .unwrap();
    assert!(
        matches!(restarted, Output::ContainerRestarted { .. }),
        "{restarted:?}"
    );
    let Output::LogLines { lines, .. } = log(50).await.unwrap() else {
        panic!("not a log")
    };
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.text == "database-started")
            .count(),
        2
    );
    // A bad count, a name that is not a core service (even a real container), one not installed.
    assert_eq!(log(0).await.unwrap_err().code, ErrorCode::InvalidArguments);
    for name in ["caddy", "agent", &db, "../etc", "bp-db"] {
        let e = restart_node_service_of(
            context.clone(),
            Operation::RestartNodeService { name: name.into() },
            &containers,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{name}");
    }
    let e = restart_node_service_of(
        context.clone(),
        Operation::RestartNodeService {
            name: "sftp".into(),
        },
        &containers,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);

    for name in [&db, &pma] {
        docker
            .remove(&Name::parse(name).unwrap(), true)
            .await
            .unwrap();
    }
}
