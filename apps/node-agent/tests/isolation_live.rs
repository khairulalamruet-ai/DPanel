//! Tenant isolation for real (T19.03): two tenants on one box, and every way across that
//! the platform could have left open - files, network and processes. (The database side is
//! in `db_live.rs`: one tenant's user cannot read another's database, and the other tenant's
//! network does not even reach the server; the panel's API side is `security_tests`.)
//! Ignored by default; run by `deploy/dev/isolation-live.sh` (needs the helper and Docker).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{
    ContainerSpec, DockerManager, ImageRef, Name, Network, VolumeMount,
};
use builtpanel_node_agent::ops::tenant::{TenantContext, create_tenant};
use builtpanel_node_protocol::{Operation, Output};
use uuid::Uuid;

const BUSYBOX: &str =
    "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

struct Tenant {
    uid: u32,
    username: String,
    root: PathBuf,
    network: Name,
    volume: Name,
}

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

async fn make(context: &Arc<TenantContext>, uid: u32, username: String) -> Tenant {
    let Output::TenantCreated {
        root,
        network,
        volume,
    } = create_tenant(
        context.clone(),
        Operation::CreateTenant {
            tenant_id: Uuid::now_v7(),
            username: username.clone(),
            uid,
        },
    )
    .await
    .expect("tenant created")
    else {
        panic!("unexpected output")
    };
    Tenant {
        uid,
        username,
        root: PathBuf::from(root),
        network: Name::parse(&network).unwrap(),
        volume: Name::parse(&volume).unwrap(),
    }
}

/// A container the tenant's own id runs, on `network`, with `volumes`, running `script`.
fn spec(
    who: &Tenant,
    name: &str,
    network: &Name,
    volumes: &[&Name],
    script: &str,
) -> ContainerSpec {
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-{}-{name}", who.username)).unwrap(),
        ImageRef::parse(BUSYBOX).unwrap(),
    );
    spec.command = vec!["sh".into(), "-c".into(), script.into()];
    spec.user = Some((who.uid, who.uid));
    spec.network = Network::Named(network.clone());
    for volume in volumes {
        spec.volumes.push(VolumeMount {
            volume: (*volume).clone(),
            target: "/data".into(),
            read_only: false,
        });
    }
    spec
}

async fn run(docker: &DockerManager, spec: &ContainerSpec) -> (i64, String) {
    let result = docker
        .run_to_end(spec, Duration::from_secs(30))
        .await
        .unwrap();
    (result.exit_code, result.output)
}

#[tokio::test]
#[ignore = "needs the privileged helper running as root and Docker"]
async fn one_tenant_cannot_reach_another_tenants_files_network_or_processes() {
    let (Some(helper_socket), Some(tenants_root)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var_os("BUILTPANEL_TEST_TENANTS_ROOT"),
    ) else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: PathBuf::from(helper_socket),
        tenants_root: PathBuf::from(tenants_root),
        docker: docker.clone(),
        vhosts_root: std::env::temp_dir().join(format!("bp-vhosts-{}", std::process::id())),
    });
    docker
        .pull_image(&ImageRef::parse(BUSYBOX).unwrap())
        .await
        .unwrap();
    let pid = std::process::id();
    let alice = make(&context, 10_060, format!("ia{pid}")).await;
    let bob = make(&context, 10_061, format!("ib{pid}")).await;

    // Alice keeps a secret and runs a service (a listener, and a process with a telltale name).
    let (code, _) = run(
        &docker,
        &spec(
            &alice,
            "seed",
            &alice.network,
            &[&alice.volume],
            "echo secret > /data/secret.txt && chmod 600 /data/secret.txt",
        ),
    )
    .await;
    assert_eq!(code, 0, "alice could not write her own file");
    let server = spec(
        &alice,
        "srv",
        &alice.network,
        &[&alice.volume],
        "hostname -i > /data/ip; sleep 4242 & while true; do echo hello | nc -l -p 8080; done",
    );
    docker.create_container(&server).await.unwrap();
    docker.start(&server.name).await.unwrap();
    let ip_file = alice.root.join("www/ip");
    let mut ip = String::new();
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&ip_file)
            && !text.trim().is_empty()
        {
            ip = text.trim().to_owned();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!ip.is_empty(), "alice's service never reported its address");
    let server_name = server.name.as_str().to_owned();

    // Control: from alice's own network the service is there (the attacks below mean something).
    let (code, out) = run(
        &docker,
        &spec(
            &alice,
            "ctl",
            &alice.network,
            &[],
            &format!("nc -w 3 {server_name} 8080 < /dev/null"),
        ),
    )
    .await;
    assert_eq!((code, out.trim()), (0, "hello"), "control: {out}");

    // FILES: bob's id, with alice's volume mounted by mistake or malice, gets nothing.
    let (code, out) = run(
        &docker,
        &spec(
            &bob,
            "steal",
            &bob.network,
            &[&alice.volume],
            "cat /data/secret.txt",
        ),
    )
    .await;
    assert_ne!(code, 0, "bob read alice's file: {out}");
    assert!(
        out.lines().all(|l| l.trim() != "secret"),
        "the content leaked: {out}"
    );
    let (code, out) = run(
        &docker,
        &spec(
            &bob,
            "plant",
            &bob.network,
            &[&alice.volume],
            "echo owned > /data/planted.txt; ls /data",
        ),
    )
    .await;
    assert_ne!(code, 0, "bob wrote into alice's folder: {out}");
    assert!(!alice.root.join("www/planted.txt").exists());
    // And from the host side the folder belongs to alice's id alone (0750).
    {
        use std::os::unix::fs::MetadataExt as _;
        let a = std::fs::metadata(&alice.root).unwrap();
        let b = std::fs::metadata(&bob.root).unwrap();
        assert_ne!(a.uid(), b.uid());
        assert_eq!(a.mode() & 0o007, 0, "others can enter alice's folder");
    }

    // NETWORK: bob's network does not reach alice's service, by name or by address.
    for target in [server_name.as_str(), ip.as_str()] {
        let (code, out) = run(
            &docker,
            &spec(
                &bob,
                "net",
                &bob.network,
                &[],
                &format!("nc -w 3 {target} 8080 < /dev/null"),
            ),
        )
        .await;
        assert_ne!(code, 0, "bob reached alice's service at {target}: {out}");
        assert!(!out.contains("hello"), "{out}");
    }

    // PROCESSES: bob sees none of alice's processes, and cannot signal them.
    let (code, out) = run(
        &docker,
        &spec(
            &bob,
            "ps",
            &bob.network,
            &[],
            "ps | grep -c 'sleep 42[4]2'; kill -9 1 2>&1 | head -1; true",
        ),
    )
    .await;
    assert_eq!(code, 0, "{out}");
    assert!(
        out.lines().next().is_some_and(|l| l.trim() == "0"),
        "bob can see alice's process: {out}"
    );
    // Alice's service survived all of it.
    assert!(docker.inspect(&server.name).await.unwrap().running);

    // Clean up.
    let _ = docker.remove(&server.name, true).await;
    for who in [&alice, &bob] {
        let _ = docker.remove_volume(&who.volume).await;
        let _ = docker.remove_network(&who.network).await;
    }
}
