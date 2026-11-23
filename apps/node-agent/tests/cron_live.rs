//! The node's cron scheduler for real (T10.02): a container that stands in for an account's
//! PHP container (named as the real one is), Docker's rootless daemon, and the agent's handlers.
//! Ignored by default; needs Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef, Tmpfs};
use builtpanel_node_agent::ops::{
    cron::{report_cron_runs, run_cron_job, run_forever, sync_cron_jobs},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{CronJobSpec, CronRun, CronStatus, ErrorCode, Operation, Output};
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

fn job(command: &str, schedule: &str, timeout: u32) -> CronJobSpec {
    CronJobSpec {
        id: Uuid::now_v7(),
        schedule: schedule.into(),
        command: command.into(),
        enabled: true,
        timeout_seconds: timeout,
    }
}

async fn runs(context: &Arc<TenantContext>, since: i64) -> Vec<CronRun> {
    match report_cron_runs(context.clone(), Operation::ReportCronRuns { since })
        .await
        .unwrap()
    {
        Output::CronRuns { runs } => runs,
        other => panic!("{other:?}"),
    }
}

async fn wait_for(
    context: &Arc<TenantContext>,
    job_id: Uuid,
    want: usize,
    secs: u64,
) -> Vec<CronRun> {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let found: Vec<CronRun> = runs(context, 0)
            .await
            .into_iter()
            .filter(|r| r.job_id == job_id)
            .collect();
        if found.len() >= want || std::time::Instant::now() > end {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn jobs_run_in_the_accounts_container_on_demand_and_on_schedule() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let username = format!("cr{}x", std::process::id());
    let container = builtpanel_tenant_stack::php_container_name(
        &builtpanel_domain::TenantUsername::parse(&username).unwrap(),
    )
    .unwrap();
    let _ = docker.remove(&container, true).await;
    let mut spec = ContainerSpec::new(container.clone(), image);
    spec.command = vec!["sleep".into(), "600".into()];
    spec.tmpfs.push(Tmpfs {
        target: "/var/www/html".into(),
        size_bytes: 1 << 20,
    });
    docker.create_container(&spec).await.unwrap();
    docker.start(&container).await.unwrap();
    let base = std::env::temp_dir().join(format!("bp-cron-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.join("tenants"),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });
    let tenant_id = Uuid::now_v7();
    let scheduler = tokio::spawn(run_forever(context.clone()));

    // Bad jobs are refused.
    for bad in [
        job("true", "nonsense", 10),
        job("true", "* * * * *", 0),
        job("", "* * * * *", 10),
    ] {
        let e = sync_cron_jobs(
            context.clone(),
            Operation::SyncCronJobs {
                tenant_id,
                username: username.clone(),
                jobs: vec![bad],
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }

    // On demand: the command's exit code and output, in the container's working folder.
    let ok = job("pwd; echo hello-from-cron", "0 0 1 1 *", 30);
    let failing = job("echo bad; exit 7", "0 0 1 1 *", 30);
    let slow = job("echo started; sleep 60", "0 0 1 1 *", 2);
    for j in [&ok, &failing, &slow] {
        let started = run_cron_job(
            context.clone(),
            Operation::RunCronJob {
                tenant_id,
                username: username.clone(),
                job: j.clone(),
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(started, Output::CronStarted { started: true }),
            "{started:?}"
        );
    }
    let r = wait_for(&context, ok.id, 1, 30).await;
    assert_eq!(
        (r[0].status, r[0].exit_code),
        (CronStatus::Ok, Some(0)),
        "{r:?}"
    );
    assert!(
        r[0].output.contains("/var/www/html") && r[0].output.contains("hello-from-cron"),
        "{r:?}"
    );
    let r = wait_for(&context, failing.id, 1, 30).await;
    assert_eq!(
        (r[0].status, r[0].exit_code),
        (CronStatus::Failed, Some(7)),
        "{r:?}"
    );
    let r = wait_for(&context, slow.id, 1, 40).await;
    assert_eq!(r[0].status, CronStatus::Timeout, "{r:?}");
    // The same job is not started again while it runs.
    let long = job("sleep 20", "0 0 1 1 *", 60);
    let first = run_cron_job(
        context.clone(),
        Operation::RunCronJob {
            tenant_id,
            username: username.clone(),
            job: long.clone(),
        },
    )
    .await
    .unwrap();
    let second = run_cron_job(
        context.clone(),
        Operation::RunCronJob {
            tenant_id,
            username: username.clone(),
            job: long.clone(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(first, Output::CronStarted { started: true }));
    assert!(
        matches!(second, Output::CronStarted { started: false }),
        "{second:?}"
    );
    assert!(
        runs(&context, 0)
            .await
            .iter()
            .any(|r| r.job_id == long.id && r.status == CronStatus::Skipped)
    );
    // Old runs are not reported when `since` is later.
    assert!(runs(&context, i64::MAX).await.is_empty());

    // On schedule: every minute; a disabled job stays quiet.
    let every = job("echo tick", "* * * * *", 30);
    let mut off = job("echo never", "* * * * *", 30);
    off.enabled = false;
    let synced = sync_cron_jobs(
        context.clone(),
        Operation::SyncCronJobs {
            tenant_id,
            username: username.clone(),
            jobs: vec![every.clone(), off.clone()],
        },
    )
    .await
    .unwrap();
    assert!(matches!(synced, Output::CronSynced { count: 2 }));
    let r = wait_for(&context, every.id, 1, 100).await;
    assert_eq!(r.len(), 1, "the job ran on its schedule");
    assert_eq!(
        (r[0].status, r[0].output.trim()),
        (CronStatus::Ok, "tick"),
        "{r:?}"
    );
    assert!(runs(&context, 0).await.iter().all(|r| r.job_id != off.id));
    // The jobs are kept in a file, for a restart.
    let kept = std::fs::read_to_string(base.join("cron.json")).unwrap();
    assert!(kept.contains(&every.id.to_string()));

    scheduler.abort();
    docker.remove(&container, true).await.unwrap();
    let _ = std::fs::remove_dir_all(&base);
}
