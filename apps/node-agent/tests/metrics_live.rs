//! `collect_metrics` for real (T10.08): thirty running containers, Docker's rootless daemon,
//! the agent's handler. Checks the figures and the cost (the roadmap asks for less than 2 % of
//! a CPU with a hundred containers, read every 30 seconds: 0.6 CPU-seconds per reading).
//! Ignored by default; needs Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Instant};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Name};
use builtpanel_node_agent::ops::{metrics::collect_metrics, tenant::TenantContext};
use builtpanel_node_protocol::{ContainerMetrics, NodeMetrics, Operation, Output};

const CONTAINERS: usize = 30;

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

/// CPU time this process has used, in milliseconds.
fn cpu_ms() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let after = stat.rsplit_once(')').unwrap().1;
    let f: Vec<&str> = after.split_whitespace().collect();
    // utime and stime are fields 14 and 15 (11 and 12 after the name).
    (f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap()) * 10
}

async fn collect(context: &Arc<TenantContext>) -> (NodeMetrics, Vec<ContainerMetrics>) {
    match collect_metrics(context.clone(), Operation::CollectMetrics)
        .await
        .unwrap()
    {
        Output::Metrics {
            node, containers, ..
        } => (node, containers),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn the_node_and_its_containers_are_measured_cheaply() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let names: Vec<Name> = (0..CONTAINERS)
        .map(|i| Name::parse(&format!("bp-mt{}x-m{i}", std::process::id())).unwrap())
        .collect();
    for (i, name) in names.iter().enumerate() {
        let mut spec = ContainerSpec::new(name.clone(), image.clone());
        // The first one burns some CPU, the second writes to its disk.
        spec.command = match i {
            0 => vec!["sh".into(), "-c".into(), "while :; do :; done".into()],
            1 => vec![
                "sh".into(),
                "-c".into(),
                "dd if=/dev/zero of=/tmp/x bs=1M count=20 2>/dev/null; sync; sleep 600".into(),
            ],
            _ => vec!["sleep".into(), "600".into()],
        };
        spec.tmpfs.push(builtpanel_docker_manager::Tmpfs {
            target: "/tmp".into(),
            size_bytes: 64 << 20,
        });
        docker.create_container(&spec).await.unwrap();
        docker.start(name).await.unwrap();
    }
    let base = std::env::temp_dir().join(format!("bp-metrics-live-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.clone(),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });

    let (node, first) = collect(&context).await;
    assert!(node.cpus >= 1 && node.cpu_total_jiffies > node.cpu_busy_jiffies);
    assert!(
        node.memory_total_bytes > node.memory_available_bytes && node.memory_available_bytes > 0
    );
    assert!(
        node.uptime_seconds > 0
            && node.disk_total_bytes >= node.disk_used_bytes
            && node.disk_used_bytes > 0
    );
    let mine: Vec<&ContainerMetrics> = first
        .iter()
        .filter(|c| {
            c.name
                .starts_with(&format!("bp-mt{}x-", std::process::id()))
        })
        .collect();
    assert_eq!(mine.len(), CONTAINERS);
    assert!(
        mine.iter().all(|c| c.memory_bytes > 0 && c.pids >= 1),
        "{mine:?}"
    );

    // Cost: five readings, CPU time of this process (the agent's part) and wall time.
    let (cpu_before, started) = (cpu_ms(), Instant::now());
    let mut last = Vec::new();
    for _ in 0..5 {
        last = collect(&context).await.1;
    }
    let (cpu, wall) = (cpu_ms() - cpu_before, started.elapsed());
    let per_reading_hundred = cpu as f64 / 5.0 * (100.0 / CONTAINERS as f64);
    eprintln!(
        "cpu per reading for 100 containers: {per_reading_hundred:.0} ms, wall per reading: {:?}",
        wall / 5
    );
    assert!(
        per_reading_hundred < 600.0,
        "{per_reading_hundred} ms of CPU per reading"
    );
    // Counters grow: the busy container used CPU between the readings.
    let busy = |list: &[ContainerMetrics]| {
        list.iter()
            .find(|c| c.name.ends_with("-m0"))
            .map(|c| c.cpu_nanos)
            .unwrap()
    };
    assert!(busy(&last) > busy(&first), "CPU time moves");
    let writer = last.iter().find(|c| c.name.ends_with("-m1")).unwrap();
    eprintln!(
        "writer io: {} read / {} written",
        writer.io_read_bytes, writer.io_write_bytes
    );

    for name in &names {
        docker.remove(name, true).await.unwrap();
    }
    let _ = std::fs::remove_dir_all(&base);
}
