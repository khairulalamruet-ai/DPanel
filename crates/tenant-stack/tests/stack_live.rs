//! The rendered stack on a real Docker daemon (the build box's rootless one).
//! Ignored by default; run with `--include-ignored` where Docker is available.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, time::Duration};

use builtpanel_docker_manager::DockerManager;
use builtpanel_domain::{TenantUsername, UidRange};
use builtpanel_tenant_stack::{Allowlist, PlanResources, Tenant, render};
use uuid::Uuid;

fn socket() -> PathBuf {
    if let Ok(path) = std::env::var("BUILTPANEL_TEST_DOCKER_SOCKET") {
        return path.into();
    }
    let uid = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        })
        .unwrap_or_default();
    [
        PathBuf::from(format!("/run/user/{uid}/docker.sock")),
        PathBuf::from("/var/run/docker.sock"),
    ]
    .into_iter()
    .find(|p| p.exists())
    .expect("no Docker socket found; set BUILTPANEL_TEST_DOCKER_SOCKET")
}

#[tokio::test]
#[ignore = "needs Docker and a registry connection for the images"]
async fn the_stack_starts_as_the_tenant_not_as_root_and_nothing_is_left_behind() {
    let list = Allowlist::embedded().unwrap();
    let manager = DockerManager::connect(&socket()).unwrap();
    for role in [
        builtpanel_tenant_stack::Role::Web,
        builtpanel_tenant_stack::Role::Php,
    ] {
        manager
            .pull_image(list.current(role).unwrap())
            .await
            .unwrap();
    }
    // A name no other run uses.
    let name = format!(
        "t{}x{}",
        std::process::id(),
        Uuid::now_v7().simple().to_string().get(20..28).unwrap()
    );
    let username = TenantUsername::parse(&name).unwrap();
    let uid = UidRange::DEFAULT.allocate([]).unwrap();
    let stack = render(
        &Tenant {
            id: Uuid::now_v7(),
            username: &username,
            uid,
            resources: PlanResources {
                cpu_millis: Some(500),
                memory_bytes: Some(512 * 1024 * 1024),
                pids: Some(128),
            },
            php_version: None,
            extra_php: &[],
        },
        &list,
    )
    .unwrap();

    manager.create_network(&stack.network, false).await.unwrap();
    for volume in &stack.volumes {
        manager.create_volume(volume).await.unwrap();
    }
    let raw = bollard::Docker::connect_with_unix(
        socket().to_str().unwrap(),
        30,
        bollard::API_DEFAULT_VERSION,
    )
    .unwrap();
    let result: Result<(), String> = async {
        for spec in &stack.services {
            manager
                .create_container(spec)
                .await
                .map_err(|e| e.to_string())?;
            manager.start(&spec.name).await.map_err(|e| e.to_string())?;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        for spec in &stack.services {
            let info = manager
                .inspect(&spec.name)
                .await
                .map_err(|e| e.to_string())?;
            assert!(
                info.running,
                "{} is not running: {info:?}",
                spec.name.as_str()
            );
            // Docker's own record: the user it runs as, and what it was not given.
            let inspected = raw
                .inspect_container(spec.name.as_str(), None)
                .await
                .map_err(|e| e.to_string())?;
            let config = inspected.config.unwrap();
            assert_eq!(
                config.user.as_deref(),
                Some(&*format!("{0}:{0}", uid.get()))
            );
            let host = inspected.host_config.unwrap();
            assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
            assert_eq!(host.readonly_rootfs, Some(true));
            assert_eq!(host.privileged, Some(false));
            // Only the web container is published, and only on the machine's loopback address (the
            // node's web server comes in that way); nothing else is.
            let published = host.port_bindings.unwrap_or_default();
            if spec.role.as_deref() == Some("web") {
                assert_eq!(published.len(), 1, "{published:?}");
                for binding in published.values().flatten().flatten() {
                    assert_eq!(binding.host_ip.as_deref(), Some("127.0.0.1"), "{binding:?}");
                }
            } else {
                assert!(published.is_empty(), "nothing published: {published:?}");
            }
            // The memory limit Docker enforces is the one in the plan.
            let status = manager.usage(&spec.name).await.map_err(|e| e.to_string())?;
            assert!(
                status.memory_limit_bytes > 0 && status.memory_limit_bytes <= 512 * 1024 * 1024
            );
        }
        Ok(())
    }
    .await;

    // Always clean up, then report.
    for spec in stack.services.iter().rev() {
        let _ = manager.remove(&spec.name, true).await;
    }
    for volume in &stack.volumes {
        let _ = manager.remove_volume(volume).await;
    }
    let _ = manager.remove_network(&stack.network).await;
    assert!(
        manager
            .list()
            .await
            .unwrap()
            .iter()
            .all(|n| !n.contains(&name)),
        "nothing left behind"
    );
    result.unwrap();
}
