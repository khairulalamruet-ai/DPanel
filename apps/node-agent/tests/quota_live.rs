//! Storage quotas for real, on an XFS filesystem mounted with project quotas
//! (the live script makes a loop-mounted one), with the privileged helper
//! running as root and a container running as the tenant.
//! Ignored by default; without BUILTPANEL_TEST_HELPER_SOCKET / _TENANTS_ROOT it
//! says so and passes.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{
    ContainerSpec, DockerManager, ImageRef, Name, Network, VolumeMount,
};
use builtpanel_node_agent::ops::tenant::{TenantContext, create_tenant, set_storage_quota};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
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

/// Runs a shell snippet as the tenant, with its volume at /data; returns the exit code.
async fn as_tenant(
    docker: &DockerManager,
    name: &str,
    uid: u32,
    volume: &str,
    script: &str,
) -> i64 {
    let net = Name::parse(&format!("bp-{name}-net")).unwrap();
    let mut spec = ContainerSpec::new(
        Name::parse(&format!(
            "bp-{name}-run{}",
            Uuid::now_v7().simple().to_string().get(24..30).unwrap()
        ))
        .unwrap(),
        ImageRef::parse(BUSYBOX).unwrap(),
    );
    spec.command = vec!["sh".into(), "-c".into(), script.into()];
    spec.user = Some((uid, uid));
    spec.network = Network::Named(net);
    spec.volumes.push(VolumeMount {
        volume: Name::parse(volume).unwrap(),
        target: "/data".into(),
        read_only: false,
    });
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
    let _ = docker.remove(&spec.name, true).await;
    exit
}

#[tokio::test]
#[ignore = "needs the privileged helper as root and an XFS filesystem with project quotas"]
async fn writing_more_than_the_quota_fails_and_raising_it_lets_the_write_through() {
    let (Some(helper_socket), Some(tenants_root)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var_os("BUILTPANEL_TEST_TENANTS_ROOT"),
    ) else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: helper_socket.into(),
        tenants_root: tenants_root.into(),
        docker: docker.clone(),
        vhosts_root: std::env::temp_dir().join(format!("bp-vhosts-{}", std::process::id())),
    });
    docker
        .pull_image(&ImageRef::parse(BUSYBOX).unwrap())
        .await
        .unwrap();

    let name = format!("q{}x", std::process::id());
    let uid = 10_071;
    let tenant_id = Uuid::now_v7();
    let Output::TenantCreated {
        volume, network, ..
    } = create_tenant(
        context.clone(),
        Operation::CreateTenant {
            tenant_id,
            username: name.clone(),
            uid,
        },
    )
    .await
    .expect("created")
    else {
        panic!()
    };
    let quota = |disk: Option<u64>, inodes: Option<u64>| Operation::SetStorageQuota {
        tenant_id,
        uid,
        disk_bytes: disk,
        inodes,
    };

    // 4 MiB and 300 files.
    let out = set_storage_quota(context.clone(), quota(Some(4 << 20), Some(1_000)))
        .await
        .expect("quota set");
    assert_eq!(
        out,
        Output::StorageQuotaSet {
            backend: "xfs".into()
        }
    );

    // Under the limit: fine.
    assert_eq!(
        as_tenant(
            &docker,
            &name,
            uid,
            &volume,
            "dd if=/dev/zero of=/data/small bs=1M count=2 2>&1"
        )
        .await,
        0
    );
    // Over the limit: the write fails (the DoD).
    let over = as_tenant(&docker, &name, uid, &volume, "dd if=/dev/zero of=/data/big bs=1M count=12 2>&1 | tail -1; test $(stat -c %s /data/big) -le 4194304").await;
    assert_eq!(over, 0, "the file must have been cut off at the limit");
    let too_big = as_tenant(
        &docker,
        &name,
        uid,
        &volume,
        "dd if=/dev/zero of=/data/big2 bs=1M count=12 2>/dev/null",
    )
    .await;
    assert_ne!(too_big, 0, "writing past the quota must fail");
    // Too many files: make room first (the disk quota is spent), then create more than allowed.
    assert_eq!(
        as_tenant(
            &docker,
            &name,
            uid,
            &volume,
            "rm -f /data/big /data/big2 /data/small"
        )
        .await,
        0
    );
    let many = as_tenant(
        &docker,
        &name,
        uid,
        &volume,
        "i=0; while [ $i -lt 1200 ]; do touch /data/f$i || exit 3; i=$((i+1)); done",
    )
    .await;
    assert_eq!(many, 3, "creating files past the file limit must fail");
    let made: usize = std::fs::read_dir(
        std::env::var_os("BUILTPANEL_TEST_TENANTS_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap()
            .join(tenant_id.to_string())
            .join("www"),
    )
    .unwrap()
    .count();
    assert!(
        (900..=1000).contains(&made),
        "stopped at the limit, made {made} files"
    );

    // Raise the limit and what failed before now works; unlimited removes it.
    set_storage_quota(context.clone(), quota(Some(64 << 20), None))
        .await
        .unwrap();
    assert_eq!(
        as_tenant(
            &docker,
            &name,
            uid,
            &volume,
            "dd if=/dev/zero of=/data/big3 bs=1M count=12 2>/dev/null"
        )
        .await,
        0
    );
    set_storage_quota(context.clone(), quota(None, None))
        .await
        .unwrap();
    assert_eq!(
        as_tenant(
            &docker,
            &name,
            uid,
            &volume,
            "dd if=/dev/zero of=/data/big4 bs=1M count=20 2>/dev/null"
        )
        .await,
        0
    );

    // Refusals.
    let err = set_storage_quota(context.clone(), quota(Some(5), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    let unknown = Operation::SetStorageQuota {
        tenant_id: Uuid::now_v7(),
        uid: 10_072,
        disk_bytes: Some(4 << 20),
        inodes: None,
    };
    assert!(
        set_storage_quota(context.clone(), unknown).await.is_err(),
        "a tenant that was never prepared"
    );

    let _ = docker.remove_volume(&Name::parse(&volume).unwrap()).await;
    let _ = docker.remove_network(&Name::parse(&network).unwrap()).await;
}
