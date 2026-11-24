//! The manager against a real Docker daemon (the build box's rootless one).
//! Ignored by default; run with `--include-ignored` on a machine with Docker.
//! The socket is `BUILTPANEL_TEST_DOCKER_SOCKET`, else the user's rootless socket,
//! else `/var/run/docker.sock`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use builtpanel_docker_manager::{
    ContainerSpec, DockerError, DockerManager, ImageRef, Name, Network, Tmpfs, VolumeMount,
};

/// busybox 1.37 (multi-arch index), pinned.
const BUSYBOX: &str =
    "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

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
    let candidates = [
        std::env::var("XDG_RUNTIME_DIR")
            .map(|d| PathBuf::from(d).join("docker.sock"))
            .unwrap_or_default(),
        PathBuf::from(format!("/run/user/{uid}/docker.sock")),
        PathBuf::from("/var/run/docker.sock"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .expect("no Docker socket found; set BUILTPANEL_TEST_DOCKER_SOCKET")
}

fn manager() -> DockerManager {
    DockerManager::connect(&socket()).unwrap()
}

/// A name nobody else uses: tests run in parallel and may be re-run.
fn unique(tag: &str) -> Name {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    Name::parse(&format!("bp-t{}-{tag}-{nanos}", std::process::id())).unwrap()
}

fn busybox() -> ImageRef {
    ImageRef::parse(BUSYBOX).unwrap()
}

async fn wait_exit(m: &DockerManager, name: &Name) -> i64 {
    for _ in 0..200 {
        let info = m.inspect(name).await.unwrap();
        if !info.running && info.status != "created" {
            return info.exit_code.expect("an exit code");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("container did not exit");
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn a_container_really_runs_with_the_hardening_applied() {
    let m = manager();
    assert!(!m.version().await.unwrap().is_empty());
    m.pull_image(&busybox()).await.unwrap();
    let (name, volume) = (unique("hard"), unique("vol"));
    m.create_volume(&volume).await.unwrap();
    m.create_volume(&volume).await.unwrap(); // again is fine

    // The container checks its own cage and reports through its exit code.
    let script = r#"
        touch /cannot 2>/dev/null && exit 10
        touch /tmp/ok || exit 11
        touch /data/ok || exit 12
        grep -Eq '^CapEff:[[:space:]]*0000000000000000$' /proc/self/status || exit 13
        grep -Eq '^NoNewPrivs:[[:space:]]*1$' /proc/self/status || exit 14
        [ "$GREETING" = "hello world" ] || exit 15
        ls /sys/class/net | grep -qv '^lo$' && exit 16
        [ -e /dev/sda ] && exit 17
        exit 0
    "#;
    let mut spec = ContainerSpec::new(name.clone(), busybox());
    spec.command = vec!["sh".into(), "-c".into(), script.into()];
    spec.env.insert("GREETING".into(), "hello world".into());
    spec.tmpfs.push(Tmpfs {
        target: "/tmp".into(),
        size_bytes: 1 << 20,
    });
    spec.volumes.push(VolumeMount {
        volume: volume.clone(),
        target: "/data".into(),
        read_only: false,
    });
    let id = m.create_container(&spec).await.unwrap();
    assert_eq!(id.len(), 64);
    let created = m.inspect(&name).await.unwrap();
    assert_eq!(
        (created.status.as_str(), created.running),
        ("created", false)
    );
    assert_eq!(created.name, name.as_str());
    assert_eq!(created.labels["builtpanel.managed"], "true");
    assert!(m.list().await.unwrap().contains(&name.as_str().to_owned()));

    m.start(&name).await.unwrap();
    assert_eq!(
        wait_exit(&m, &name).await,
        0,
        "a check inside the container failed"
    );

    // The volume is in use until the container is gone.
    assert!(matches!(
        m.remove_volume(&volume).await,
        Err(DockerError::Conflict(_))
    ));
    m.remove(&name, false).await.unwrap();
    assert!(matches!(m.inspect(&name).await, Err(DockerError::NotFound)));
    m.remove_volume(&volume).await.unwrap();
    assert!(matches!(
        m.remove_volume(&volume).await,
        Err(DockerError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn start_stop_usage_and_remove_follow_the_container_state() {
    let m = manager();
    m.pull_image(&busybox()).await.unwrap();
    let (name, net) = (unique("life"), unique("net"));
    m.create_network(&net, true).await.unwrap();
    assert!(matches!(
        m.create_network(&net, true).await,
        Err(DockerError::Conflict(_))
    ));

    let mut spec = ContainerSpec::new(name.clone(), busybox());
    spec.command = vec!["sleep".into(), "300".into()];
    spec.memory_bytes = 64 << 20;
    spec.network = Network::Named(net.clone());
    m.create_container(&spec).await.unwrap();
    // The same name again is refused.
    assert!(matches!(
        m.create_container(&spec).await,
        Err(DockerError::Conflict(_))
    ));

    m.start(&name).await.unwrap();
    m.start(&name).await.unwrap(); // already running: fine
    let info = m.inspect(&name).await.unwrap();
    assert!(info.running && info.status == "running" && info.exit_code.is_none());
    let usage = m.usage(&name).await.unwrap();
    assert_eq!(
        usage.memory_limit_bytes,
        64 << 20,
        "the limit is the one we set"
    );
    assert!(usage.pids >= 1 && usage.memory_bytes > 0);

    // A running container is not removed by accident, and its network is busy.
    assert!(matches!(
        m.remove(&name, false).await,
        Err(DockerError::Conflict(_))
    ));
    assert!(m.remove_network(&net).await.is_err());

    m.stop(&name, Duration::from_secs(1)).await.unwrap();
    m.stop(&name, Duration::from_secs(1)).await.unwrap(); // already stopped: fine
    let info = m.inspect(&name).await.unwrap();
    assert!(!info.running);
    assert!(info.exit_code.is_some());

    m.remove(&name, true).await.unwrap();
    assert!(matches!(m.start(&name).await, Err(DockerError::NotFound)));
    m.remove_network(&net).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn objects_we_did_not_create_are_never_touched() {
    use bollard::{
        Docker,
        models::{ContainerCreateBody, VolumeCreateRequest},
        query_parameters::{CreateContainerOptionsBuilder, RemoveContainerOptionsBuilder},
    };
    let m = manager();
    m.pull_image(&busybox()).await.unwrap();
    // Somebody else's container and volume, with names that look like ours.
    let raw =
        Docker::connect_with_unix(socket().to_str().unwrap(), 30, bollard::API_DEFAULT_VERSION)
            .unwrap();
    let (name, volume) = (unique("foreign"), unique("fvol"));
    raw.create_container(
        Some(
            CreateContainerOptionsBuilder::default()
                .name(name.as_str())
                .build(),
        ),
        ContainerCreateBody {
            image: Some(BUSYBOX.to_owned()),
            cmd: Some(vec!["sleep".into(), "300".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    raw.create_volume(VolumeCreateRequest {
        name: Some(volume.as_str().to_owned()),
        ..Default::default()
    })
    .await
    .unwrap();

    assert!(matches!(
        m.inspect(&name).await,
        Err(DockerError::NotManaged)
    ));
    assert!(matches!(m.start(&name).await, Err(DockerError::NotManaged)));
    assert!(matches!(
        m.stop(&name, Duration::ZERO).await,
        Err(DockerError::NotManaged)
    ));
    assert!(matches!(m.usage(&name).await, Err(DockerError::NotManaged)));
    assert!(matches!(
        m.remove(&name, true).await,
        Err(DockerError::NotManaged)
    ));
    assert!(matches!(
        m.create_volume(&volume).await,
        Err(DockerError::NotManaged)
    ));
    assert!(matches!(
        m.remove_volume(&volume).await,
        Err(DockerError::NotManaged)
    ));
    assert!(!m.list().await.unwrap().contains(&name.as_str().to_owned()));
    // Still there, untouched.
    let still = raw.inspect_container(name.as_str(), None).await.unwrap();
    assert_eq!(still.state.unwrap().running, Some(false));

    raw.remove_container(
        name.as_str(),
        Some(RemoveContainerOptionsBuilder::default().force(true).build()),
    )
    .await
    .unwrap();
    raw.remove_volume(
        volume.as_str(),
        None::<bollard::query_parameters::RemoveVolumeOptions>,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn missing_things_and_a_dead_socket_are_clear_errors() {
    let m = manager();
    let ghost = unique("ghost");
    assert!(matches!(
        m.inspect(&ghost).await,
        Err(DockerError::NotFound)
    ));
    assert!(matches!(
        m.remove(&ghost, true).await,
        Err(DockerError::NotFound)
    ));
    assert!(matches!(
        m.remove_network(&ghost).await,
        Err(DockerError::NotFound)
    ));

    // An image that is not present locally: create says so instead of pulling a tag.
    let absent = ImageRef::parse(&format!("busybox@sha256:{}", "0".repeat(64))).unwrap();
    let spec = ContainerSpec::new(unique("noimg"), absent.clone());
    assert!(matches!(
        m.create_container(&spec).await,
        Err(DockerError::NotFound)
    ));
    // And the registry does not have it either.
    assert!(m.pull_image(&absent).await.is_err());

    // A specification that is not valid never reaches Docker.
    let mut bad = ContainerSpec::new(unique("bad"), busybox());
    bad.memory_bytes = 1;
    assert!(matches!(
        m.create_container(&bad).await,
        Err(DockerError::Invalid(_))
    ));

    // No daemon behind the socket: said at once, and worth retrying later.
    let err = match DockerManager::connect(std::path::Path::new("/nonexistent/docker.sock")) {
        Err(err) => err,
        Ok(dead) => dead.version().await.unwrap_err(),
    };
    assert!(err.is_retryable(), "{err:?}");
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn a_cron_command_runs_inside_the_container_with_its_exit_code_output_and_time_limit() {
    let m = manager();
    m.pull_image(&busybox()).await.unwrap();
    let name = unique("cron");
    let mut spec = ContainerSpec::new(name.clone(), busybox());
    spec.command = vec!["sleep".into(), "120".into()];
    spec.tmpfs.push(Tmpfs {
        target: "/work".into(),
        size_bytes: 1 << 20,
    });
    m.create_container(&spec).await.unwrap();
    // Not running: refused.
    assert!(matches!(
        m.run_user_command(&name, "true", "/work", Duration::from_secs(5))
            .await,
        Err(DockerError::Conflict(_))
    ));
    m.start(&name).await.unwrap();
    let run = |cmd: &'static str, secs: u64| {
        let (m, name) = (m.clone(), name.clone());
        async move {
            m.run_user_command(&name, cmd, "/work", Duration::from_secs(secs))
                .await
                .unwrap()
        }
    };
    let ok = run("pwd; echo out; echo err >&2", 10).await;
    assert_eq!(ok.exit_code, 0);
    assert!(
        ok.output.contains("/work") && ok.output.contains("out") && ok.output.contains("err"),
        "{}",
        ok.output
    );
    let failed = run("echo nope; exit 3", 10).await;
    assert_eq!((failed.exit_code, failed.output.trim()), (3, "nope"));
    // Pipes and variables are the shell's.
    assert_eq!(run("echo $((6*7)) | tr 4 5", 10).await.output.trim(), "52");
    // The time limit kills it.
    let started = std::time::Instant::now();
    let slow = run("sleep 60", 2).await;
    assert_eq!(slow.exit_code, 137, "{slow:?}");
    assert!(started.elapsed() < Duration::from_secs(30));
    // A lot of output keeps its end.
    let loud = run(
        "i=0; while [ $i -lt 3000 ]; do echo line-$i-padding-padding; i=$((i+1)); done",
        20,
    )
    .await;
    assert!(
        loud.output.len() <= 8 * 1024 && loud.output.contains("line-2999-"),
        "{}",
        loud.output.len()
    );
    // Bad arguments.
    assert!(matches!(
        m.run_user_command(&name, "a\0b", "/work", Duration::from_secs(5))
            .await,
        Err(DockerError::Conflict(_))
    ));
    assert!(matches!(
        m.run_user_command(&name, "true", "work", Duration::from_secs(5))
            .await,
        Err(DockerError::Conflict(_))
    ));
    m.remove(&name, true).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn every_container_rotates_its_log_so_it_stays_within_30_mib() {
    let m = manager();
    m.pull_image(&busybox()).await.unwrap();
    let name = unique("rot");
    let mut spec = ContainerSpec::new(name.clone(), busybox());
    spec.command = vec!["sleep".into(), "30".into()];
    m.create_container(&spec).await.unwrap();
    let docker = bollard::Docker::connect_with_unix(
        socket().to_str().unwrap(),
        30,
        bollard::API_DEFAULT_VERSION,
    )
    .unwrap();
    let info = docker
        .inspect_container(
            name.as_str(),
            None::<bollard::query_parameters::InspectContainerOptions>,
        )
        .await
        .unwrap();
    let log = info.host_config.unwrap().log_config.unwrap();
    assert_eq!(log.typ.as_deref(), Some("json-file"));
    let config = log.config.unwrap();
    assert_eq!(config.get("max-size").map(String::as_str), Some("10m"));
    assert_eq!(config.get("max-file").map(String::as_str), Some("3"));
    m.remove(&name, true).await.unwrap();
}
