//! `list_containers` and `restart_container` for real (T10.12): containers of two accounts
//! and one of the node, Docker's rootless daemon, the agent's handlers. Ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name};
use builtpanel_node_agent::ops::{
    services::{list_containers, restart_container},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
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

#[tokio::test]
#[ignore = "needs Docker"]
async fn an_account_lists_and_restarts_only_its_own_containers() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let pid = std::process::id();
    let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
    let make = |name: String, tenant: Option<Uuid>, role: &'static str| {
        let (docker, image) = (docker.clone(), image.clone());
        async move {
            let name = Name::parse(&name).unwrap();
            let _ = docker.remove(&name, true).await;
            let mut spec = ContainerSpec::new(name.clone(), image);
            // It prints a number that is new at every start: a restart is seen from outside.
            spec.command = vec![
                "sh".into(),
                "-c".into(),
                "echo started-$(date +%N); sleep 600".into(),
            ];
            spec.tenant = tenant;
            spec.role = Some(role.to_owned());
            docker.create_container(&spec).await.unwrap();
            docker.start(&name).await.unwrap();
            name
        }
    };
    let a_web = make(format!("bp-sva{pid}-web"), Some(alice), "web").await;
    let a_php = make(format!("bp-sva{pid}-php"), Some(alice), "php").await;
    let b_web = make(format!("bp-svb{pid}-web"), Some(bob), "web").await;
    let node_db = make(format!("bp-svdb{pid}"), None, "db").await;
    let base = std::env::temp_dir().join(format!("bp-services-live-{pid}"));
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.join("tenants"),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });

    // Alice sees her two, with their roles, and nothing of Bob's or the node's.
    let Output::Containers { items } = list_containers(
        context.clone(),
        Operation::ListContainers { tenant_id: alice },
    )
    .await
    .unwrap() else {
        panic!("not a list")
    };
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, [a_php.as_str(), a_web.as_str()]);
    assert_eq!(
        items.iter().map(|i| i.role.as_str()).collect::<Vec<_>>(),
        ["php", "web"]
    );
    assert!(
        items
            .iter()
            .all(|i| i.state == "running" && i.created > 1_700_000_000)
    );
    // An account with none gets an empty list.
    let Output::Containers { items } = list_containers(
        context.clone(),
        Operation::ListContainers {
            tenant_id: Uuid::now_v7(),
        },
    )
    .await
    .unwrap() else {
        panic!("not a list")
    };
    assert!(items.is_empty());

    // Restart: her own container is stopped and started again (a new start time).
    let restarted = restart_container(
        context.clone(),
        Operation::RestartContainer {
            tenant_id: alice,
            name: a_web.as_str().to_owned(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(&restarted, Output::ContainerRestarted { name, .. } if name == a_web.as_str()),
        "{restarted:?}"
    );
    assert!(docker.inspect(&a_web).await.unwrap().running);
    // It printed "started-..." at each start: two lines now, one in the other container.
    let starts = |name: Name| {
        let docker = docker.clone();
        async move {
            docker
                .logs(&name, 0, 100)
                .await
                .unwrap()
                .lines
                .iter()
                .filter(|l| l.text.starts_with("started-"))
                .count()
        }
    };
    // The log catches up a moment after the start.
    let mut seen = 0;
    for _ in 0..20 {
        seen = starts(a_web.clone()).await;
        if seen == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(seen, 2);
    assert_eq!(starts(a_php.clone()).await, 1);

    // Another account's container, the node's, one that does not exist and a bad name:
    // all refused alike, nothing restarted.
    let b_before = docker.inspect(&b_web).await.unwrap();
    for name in [
        b_web.as_str().to_owned(),
        node_db.as_str().to_owned(),
        format!("bp-nobody{pid}-web"),
        "../etc".to_owned(),
    ] {
        let e = restart_container(
            context.clone(),
            Operation::RestartContainer {
                tenant_id: alice,
                name: name.clone(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{name}: {e:?}");
    }
    let b_after = docker.inspect(&b_web).await.unwrap();
    assert!(b_after.running && b_after.id == b_before.id);

    for name in [&a_web, &a_php, &b_web, &node_db] {
        docker.remove(name, true).await.unwrap();
    }
}
