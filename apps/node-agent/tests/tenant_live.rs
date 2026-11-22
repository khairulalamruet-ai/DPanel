//! `create_tenant` for real: the privileged helper (run as root by the live
//! script), the agent's handler, and the box's rootless Docker.
//! Ignored by default. Needs BUILTPANEL_TEST_HELPER_SOCKET and
//! BUILTPANEL_TEST_TENANTS_ROOT (and the helper started with the same root and
//! with this user as its agent); see the script that runs it.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{os::unix::fs::MetadataExt as _, path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{
    ContainerSpec, DockerManager, ImageRef, Name, Network, VolumeMount,
};
use builtpanel_node_agent::ops::tenant::{TenantContext, create_tenant};
use builtpanel_node_protocol::{Operation, Output};
use uuid::Uuid;

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

#[tokio::test]
#[ignore = "needs the privileged helper running as root and Docker"]
async fn the_directories_the_user_id_and_the_network_are_created_and_usable() {
    // Needs a helper running as root; without one (the ordinary test run) there is
    // nothing to test here.
    let (Some(helper_socket), Some(tenants_root)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var_os("BUILTPANEL_TEST_TENANTS_ROOT"),
    ) else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let (helper_socket, tenants_root) = (PathBuf::from(helper_socket), PathBuf::from(tenants_root));
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket,
        tenants_root: tenants_root.clone(),
        docker: docker.clone(),
        vhosts_root: std::env::temp_dir().join(format!("bp-vhosts-{}", std::process::id())),
    });
    docker
        .pull_image(&ImageRef::parse(BUSYBOX).unwrap())
        .await
        .unwrap();

    let tenant_id = Uuid::now_v7();
    let username = format!("t{}x", std::process::id());
    let op = Operation::CreateTenant {
        tenant_id,
        username: username.clone(),
        uid: 10_042,
    };
    let output = create_tenant(context.clone(), op.clone())
        .await
        .expect("created");
    let Output::TenantCreated {
        root,
        network,
        volume,
    } = output.clone()
    else {
        panic!("unexpected output {output:?}")
    };
    assert_eq!(
        root,
        tenants_root
            .join(tenant_id.to_string())
            .display()
            .to_string()
    );
    assert_eq!(
        (network.as_str(), volume.as_str()),
        (
            format!("bp-{username}-net").as_str(),
            format!("bp-{username}-www").as_str()
        )
    );

    // On disk: owned by the host id of tenant id 10042, private to it and the agent's group.
    let agent = std::env::var("BUILTPANEL_TEST_AGENT_USER").unwrap_or_else(|_| "dev".into());
    let start: u32 = std::fs::read_to_string("/etc/subuid")
        .unwrap()
        .lines()
        .find_map(|l| {
            let mut fields = l.split(':');
            (fields.next() == Some(agent.as_str())).then(|| fields.next().unwrap().parse().unwrap())
        })
        .expect("subuid range of the agent user");
    for dir in [PathBuf::from(&root), PathBuf::from(&root).join("www")] {
        let meta = std::fs::metadata(&dir).unwrap();
        assert_eq!(meta.uid(), start + 10_042 - 1, "{dir:?}");
        assert_eq!(meta.mode() & 0o777, 0o750, "{dir:?}");
    }

    // The network and the volume exist, are ours, and the volume is that directory.
    let net = Name::parse(&network).unwrap();
    assert!(
        docker.ensure_network(&net, false).await.is_ok(),
        "ensure is idempotent"
    );

    // Repeating the operation changes nothing and is not an error.
    assert_eq!(create_tenant(context.clone(), op).await.unwrap(), output);

    // The tenant can really use it: a container running as that id writes into the volume.
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-{username}-probe")).unwrap(),
        ImageRef::parse(BUSYBOX).unwrap(),
    );
    spec.command = vec![
        "sh".into(),
        "-c".into(),
        "id -u && echo hello > /data/probe.txt && cat /data/probe.txt".into(),
    ];
    spec.user = Some((10_042, 10_042));
    spec.network = Network::Named(net.clone());
    spec.volumes.push(VolumeMount {
        volume: Name::parse(&volume).unwrap(),
        target: "/data".into(),
        read_only: false,
    });
    docker.create_container(&spec).await.unwrap();
    docker.start(&spec.name).await.unwrap();
    let mut exit = None;
    for _ in 0..100 {
        let info = docker.inspect(&spec.name).await.unwrap();
        if !info.running && info.status != "created" {
            exit = info.exit_code;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        exit,
        Some(0),
        "the tenant could not write to its own volume"
    );
    let written = std::fs::read_to_string(PathBuf::from(&root).join("www/probe.txt")).unwrap();
    assert_eq!(written.trim(), "hello");
    let file_owner = std::fs::metadata(PathBuf::from(&root).join("www/probe.txt"))
        .unwrap()
        .uid();
    assert_eq!(
        file_owner,
        start + 10_042 - 1,
        "files the tenant makes belong to its host id"
    );

    // A second tenant cannot use the first one's directory: other id, other owner.
    let other = Operation::CreateTenant {
        tenant_id: Uuid::now_v7(),
        username: format!("u{}x", std::process::id()),
        uid: 10_043,
    };
    let Output::TenantCreated {
        root: other_root, ..
    } = create_tenant(context.clone(), other.clone()).await.unwrap()
    else {
        panic!()
    };
    assert_ne!(
        std::fs::metadata(&other_root).unwrap().uid(),
        std::fs::metadata(&root).unwrap().uid()
    );

    // Clean up what this test made (the script removes the directories).
    let _ = docker.remove(&spec.name, true).await;
    for name in [volume.clone(), format!("bp-u{}x-www", std::process::id())] {
        let _ = docker.remove_volume(&Name::parse(&name).unwrap()).await;
    }
    let _ = docker.remove_network(&net).await;
    let _ = docker
        .remove_network(&Name::parse(&format!("bp-u{}x-net", std::process::id())).unwrap())
        .await;
}
