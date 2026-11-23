//! `list_processes` and `signal_process` for real (T10.14): containers of two accounts,
//! Docker's rootless daemon, the agent's handlers. Ignored by default; needs Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name};
use builtpanel_node_agent::ops::{
    services::{list_processes, signal_process},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output, ProcessItem, SignalOutcome};
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

async fn processes(context: &Arc<TenantContext>, tenant_id: Uuid) -> Vec<ProcessItem> {
    match list_processes(context.clone(), Operation::ListProcesses { tenant_id })
        .await
        .unwrap()
    {
        Output::Processes { items } => items,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn an_account_sees_and_ends_only_its_own_processes() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let pid = std::process::id();
    let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
    // Alice's container runs a few sleepers, one of which ignores SIGTERM; Bob's runs one.
    let script_a = "sleep 111 & sleep 222 & (trap '' TERM; exec sleep 333) & sleep 600";
    let make = |name: String, tenant: Uuid, script: &'static str| {
        let (docker, image) = (docker.clone(), image.clone());
        async move {
            let name = Name::parse(&name).unwrap();
            let _ = docker.remove(&name, true).await;
            let mut spec = ContainerSpec::new(name.clone(), image);
            spec.command = vec!["sh".into(), "-c".into(), script.into()];
            spec.tenant = Some(tenant);
            spec.role = Some("php".into());
            docker.create_container(&spec).await.unwrap();
            docker.start(&name).await.unwrap();
            name
        }
    };
    let a = make(format!("bp-pra{pid}-php"), alice, script_a).await;
    let b = make(format!("bp-prb{pid}-php"), bob, "sleep 999").await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let base = std::env::temp_dir().join(format!("bp-processes-live-{pid}"));
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.join("tenants"),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });
    let signal = |container: &Name, pid: u32, kill: bool, escalate: bool| {
        let (context, container) = (context.clone(), container.as_str().to_owned());
        async move {
            signal_process(
                context,
                Operation::SignalProcess {
                    tenant_id: alice,
                    container,
                    pid,
                    kill,
                    escalate,
                },
            )
            .await
        }
    };

    // Alice sees her own processes (and only in her container), with command lines.
    let mine = processes(&context, alice).await;
    assert!(mine.iter().all(|p| p.container == a.as_str()), "{mine:?}");
    let find = |needle: &str| mine.iter().find(|p| p.command == needle).map(|p| p.pid);
    let (p111, p222, p333) = (
        find("sleep 111").unwrap(),
        find("sleep 222").unwrap(),
        find("sleep 333").unwrap(),
    );
    assert!(mine.iter().any(|p| p.pid == 1), "{mine:?}");
    assert!(
        !mine.iter().any(|p| p.command.contains("999")),
        "Bob's process is not listed"
    );
    assert!(processes(&context, Uuid::now_v7()).await.is_empty());

    // SIGTERM ends an ordinary process.
    let done = signal(&a, p111, false, false).await.unwrap();
    assert!(
        matches!(
            done,
            Output::ProcessSignalled {
                outcome: SignalOutcome::Exited,
                ..
            }
        ),
        "{done:?}"
    );
    // One that ignores it stays (and the answer says so); SIGKILL ends it.
    let stays = signal(&a, p333, false, false).await.unwrap();
    assert!(
        matches!(
            stays,
            Output::ProcessSignalled {
                outcome: SignalOutcome::StillRunning,
                ..
            }
        ),
        "{stays:?}"
    );
    let killed = signal(&a, p333, true, false).await.unwrap();
    assert!(
        matches!(
            killed,
            Output::ProcessSignalled {
                outcome: SignalOutcome::Killed,
                ..
            }
        ),
        "{killed:?}"
    );
    // Gone processes are not found.
    let none = signal(&a, p111, false, false).await.unwrap();
    assert!(
        matches!(
            none,
            Output::ProcessSignalled {
                outcome: SignalOutcome::NotFound,
                ..
            }
        ),
        "{none:?}"
    );
    // The listing no longer has them.
    let after = processes(&context, alice).await;
    // An ended process that its parent has not collected yet is listed, as a zombie.
    assert!(
        after
            .iter()
            .all(|p| (p.pid != p111 && p.pid != p333) || p.state.starts_with('Z'))
    );
    assert!(after.iter().any(|p| p.pid == p222));

    // Process 1 and 0, another account's container, and a name that is not a container.
    for (container, process) in [
        (a.as_str().to_owned(), 1u32),
        (a.as_str().to_owned(), 0),
        (b.as_str().to_owned(), p222),
        ("../etc".to_owned(), 5),
    ] {
        let e = signal_process(
            context.clone(),
            Operation::SignalProcess {
                tenant_id: alice,
                container: container.clone(),
                pid: process,
                kill: true,
                escalate: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{container} {process}");
    }
    assert!(docker.inspect(&b).await.unwrap().running);
    assert!(
        processes(&context, bob)
            .await
            .iter()
            .any(|p| p.command.contains("999"))
    );

    // Escalation: a process that ignores SIGTERM is killed after ten seconds.
    let (context2, a2) = (context.clone(), a.clone());
    docker
        .run_user_command(
            &a,
            "(trap '' TERM; exec sleep 444) >/dev/null 2>&1 &",
            "/",
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    let p444 = processes(&context2, alice)
        .await
        .iter()
        .find(|p| p.command == "sleep 444")
        .map(|p| p.pid)
        .unwrap();
    let started = std::time::Instant::now();
    let out = signal_process(
        context2.clone(),
        Operation::SignalProcess {
            tenant_id: alice,
            container: a2.as_str().to_owned(),
            pid: p444,
            kill: false,
            escalate: true,
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(
            out,
            Output::ProcessSignalled {
                outcome: SignalOutcome::Killed,
                ..
            }
        ),
        "{out:?}"
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(9));

    docker.remove(&a, true).await.unwrap();
    docker.remove(&b, true).await.unwrap();
}
