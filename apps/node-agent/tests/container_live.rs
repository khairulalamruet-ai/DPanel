//! A container of an approved image for real (T12.12): pulled by digest, run on the tenant's
//! network, reached by name, replaced when set again, removed with its data. Needs rootless
//! Docker and the network; ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, Name, Network};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    runtimeapp::{remove_runtime_app, set_runtime_app},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output, RuntimeEnv};
use builtpanel_tenant_stack::{Allowlist, Role, network_name};
use uuid::Uuid;

const NGINX: &str = "nginxinc/nginx-unprivileged@sha256:f5116ed4606f3eb015dd4a8e28cb9b351476dc0ad4bc6e9f93b19cec130cdf03";

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

fn set(name: &str, image: Option<&str>, port: u16, memory_mb: u32) -> Operation {
    Operation::SetRuntimeApp {
        app_id: Uuid::now_v7(),
        tenant_id: Uuid::now_v7(),
        username: "carol".into(),
        uid: 10_000,
        kind: "container".into(),
        version: "0".into(),
        name: name.into(),
        dir: String::new(),
        command: Vec::new(),
        install: false,
        port,
        memory_mb,
        env: Some(RuntimeEnv(vec![("GREETING".into(), "hello".into())])),
        image: image.map(str::to_owned),
    }
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn an_approved_image_runs_on_the_tenants_network_and_goes_away_with_its_data() {
    let base = std::env::temp_dir().join("bp-container-live");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("state/vhosts")).unwrap();
    let username = TenantUsername::parse("carol").unwrap();
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let network = network_name(&username).unwrap();
    for n in ["bp-carol-app-web", "bp-ctprobe"] {
        let _ = docker.remove(&Name::parse(n).unwrap(), true).await;
    }
    let data = Name::parse("bp-carol-app-web-data").unwrap();
    let _ = docker.remove_volume(&data).await;
    docker.ensure_network(&network, false).await.unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: base.join("tenants"),
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: base.join("state/vhosts"),
    });

    // Bad input is refused before anything is pulled or started: no image, an image that is a
    // tag (it can move), a port that needs a privilege, a name with a capital, too little memory.
    for bad in [
        set("web", None, 8080, 128),
        set("web", Some("nginx:latest"), 8080, 128),
        set("web", Some("nginx@sha256:abc"), 8080, 128),
        set("web", Some(NGINX), 80, 128),
        set("Web", Some(NGINX), 8080, 128),
        set("web", Some(NGINX), 8080, 8),
    ] {
        let err = set_runtime_app(context.clone(), bad).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{err:?}");
    }
    assert!(
        docker
            .inspect(&Name::parse("bp-carol-app-web").unwrap())
            .await
            .is_err(),
        "a refused request started nothing"
    );

    let out = set_runtime_app(context.clone(), set("web", Some(NGINX), 8080, 128))
        .await
        .unwrap();
    assert_eq!(
        out,
        Output::RuntimeAppSet {
            host: "bp-carol-app-web:8080".into()
        }
    );
    // It runs from the pinned image, with the memory it was given.
    let info = docker
        .inspect(&Name::parse("bp-carol-app-web").unwrap())
        .await
        .unwrap();
    assert!(info.running, "{info:?}");
    assert_eq!(info.image, NGINX, "{info:?}");
    assert_eq!(info.limits.memory_bytes, 128 * 1024 * 1024, "{info:?}");
    // Reached by name from another container of the tenant's network.
    let allowlist = Allowlist::embedded().unwrap();
    let node_image = allowlist.runtime_image(Role::Node, "24").unwrap().clone();
    docker.pull_image(&node_image).await.unwrap();
    let mut probe = ContainerSpec::new(Name::parse("bp-ctprobe").unwrap(), node_image);
    probe.entrypoint = vec!["node".into(), "-e".into()];
    probe.command = vec![
        "fetch('http://bp-carol-app-web:8080/').then(r => r.text()).then(t => console.log('ANSWER=' + t.slice(0, 200))).catch(e => console.log('ERROR=' + e))".into(),
    ];
    probe.network = Network::Named(network.clone());
    let said = docker
        .run_to_end(&probe, Duration::from_secs(60))
        .await
        .unwrap()
        .output;
    assert!(said.contains("Welcome to nginx"), "{said}");

    // Set again, it is the same container name with the new settings (replaced, not doubled).
    let again = set_runtime_app(context.clone(), set("web", Some(NGINX), 8080, 256))
        .await
        .unwrap();
    assert!(matches!(again, Output::RuntimeAppSet { .. }));

    // Removed, the container and its data volume are gone, and removing again is not an error.
    let removed = remove_runtime_app(
        context.clone(),
        Operation::RemoveRuntimeApp {
            tenant_id: Uuid::now_v7(),
            username: "carol".into(),
            name: "web".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(removed, Output::RuntimeAppRemoved { existed: true });
    assert!(
        docker
            .inspect(&Name::parse("bp-carol-app-web").unwrap())
            .await
            .is_err()
    );
    let removed = remove_runtime_app(
        context,
        Operation::RemoveRuntimeApp {
            tenant_id: Uuid::now_v7(),
            username: "carol".into(),
            name: "web".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(removed, Output::RuntimeAppRemoved { existed: false });
}
