//! The node's cron scheduler (T10.02): runs the cron jobs of the accounts on this node inside
//! their PHP containers, and remembers the last hour of results for the control plane to
//! collect (`report_cron_runs`).
//!
//! * The jobs come with `sync_cron_jobs` (the whole list of an account, replacing the old
//!   one) and are kept in a file in the state folder, so a restart does not forget them.
//! * Every ten seconds the scheduler looks at each enabled job: when its schedule has a
//!   moment between the last look and now, it runs, in a task of its own. A job whose last
//!   run is still going is not started again (a `skipped` run is recorded). After a pause
//!   (a restart, a sleeping node) nothing is made up: a job runs once at most for the pause.
//! * The command runs with `sh -c` as the container's user in `/var/www/html`, stopped at the
//!   job's timeout (`timeout -s KILL`).

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use builtpanel_backup::schedule::Cron;
use builtpanel_node_protocol::{
    AgentError, CronJobSpec, CronRun, CronStatus, MAX_CRON_JOBS, MAX_CRON_RUNS_PER_REPORT,
    Operation, Output,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    sqlimport::{failed, invalid},
    tenant::TenantContext,
};

const KEEP_RUNS_SECS: i64 = 3600;
const MAX_KEPT_RUNS: usize = 5000;
const TICK: Duration = Duration::from_secs(10);
const WORKDIR: &str = "/var/www/html";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TenantJobs {
    username: String,
    jobs: Vec<CronJobSpec>,
}

#[derive(Default)]
struct State {
    loaded: bool,
    tenants: BTreeMap<Uuid, TenantJobs>,
    /// When each job was last looked at (so that a moment is acted on once).
    looked_at: BTreeMap<Uuid, i64>,
    running: HashSet<Uuid>,
    runs: VecDeque<CronRun>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(path: &Path, f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let state = guard.get_or_insert_with(State::default);
    if !state.loaded {
        state.loaded = true;
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(tenants) = serde_json::from_str(&text)
        {
            state.tenants = tenants;
        }
    }
    f(state)
}

fn file_of(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("cron.json")
}

fn save(path: &Path, tenants: &BTreeMap<Uuid, TenantJobs>) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(tenants)?)?;
    std::fs::rename(&tmp, path)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn check(job: &CronJobSpec) -> Result<(), AgentError> {
    Cron::parse(&job.schedule).map_err(|e| invalid(&format!("a schedule: {e}")))?;
    if job.command.is_empty() || job.command.len() > 10_000 || job.command.contains('\0') {
        return Err(invalid("a command is not usable"));
    }
    if !(1..=86_400).contains(&job.timeout_seconds) {
        return Err(invalid("a timeout is out of range"));
    }
    Ok(())
}

/// `sync_cron_jobs`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable) or the state file failing (retryable).
pub async fn sync_cron_jobs(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SyncCronJobs {
        tenant_id,
        username,
        jobs,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    builtpanel_domain::TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if jobs.len() > MAX_CRON_JOBS {
        return Err(invalid("too many cron jobs"));
    }
    let mut seen = HashSet::new();
    for job in &jobs {
        check(job)?;
        if !seen.insert(job.id) {
            return Err(invalid("a job is listed twice"));
        }
    }
    let count = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let path = file_of(&context);
    let saved = with_state(&path, |s| {
        let ids: HashSet<Uuid> = jobs.iter().map(|j| j.id).collect();
        // A job that is new, or whose schedule changed, is looked at from now on.
        let old: BTreeMap<Uuid, String> = s
            .tenants
            .get(&tenant_id)
            .map(|t| t.jobs.iter().map(|j| (j.id, j.schedule.clone())).collect())
            .unwrap_or_default();
        for job in &jobs {
            if old.get(&job.id) != Some(&job.schedule) {
                s.looked_at.insert(job.id, now());
            }
        }
        for gone in old.keys().filter(|id| !ids.contains(id)) {
            s.looked_at.remove(gone);
        }
        if jobs.is_empty() {
            s.tenants.remove(&tenant_id);
        } else {
            s.tenants.insert(tenant_id, TenantJobs { username, jobs });
        }
        save(&path, &s.tenants)
    });
    saved.map_err(|e| failed(&format!("cannot keep the cron jobs: {}", e.kind())).retryable())?;
    Ok(Output::CronSynced { count })
}

/// `run_cron_job`.
///
/// # Errors
/// [`AgentError`]: bad arguments.
pub async fn run_cron_job(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RunCronJob {
        tenant_id,
        username,
        job,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    builtpanel_domain::TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    check(&job)?;
    let started = start(&context, tenant_id, &username, job);
    Ok(Output::CronStarted { started })
}

/// `report_cron_runs`.
///
/// # Errors
/// [`AgentError`]: none in practice.
pub async fn report_cron_runs(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReportCronRuns { since } = op else {
        return Err(super::wrong_operation());
    };
    let path = file_of(&context);
    let runs = with_state(&path, |s| {
        s.runs
            .iter()
            .filter(|r| r.started_at >= since)
            .take(MAX_CRON_RUNS_PER_REPORT)
            .cloned()
            .collect()
    });
    Ok(Output::CronRuns { runs })
}

fn record(path: &Path, run: CronRun) {
    with_state(path, |s| {
        s.runs.push_back(run);
        let oldest = now() - KEEP_RUNS_SECS;
        while s
            .runs
            .front()
            .is_some_and(|r| r.started_at < oldest || s.runs.len() > MAX_KEPT_RUNS)
        {
            s.runs.pop_front();
        }
    });
}

/// Starts a run in a task of its own; `false` (and a `skipped` run) when the job is running.
fn start(context: &Arc<TenantContext>, tenant_id: Uuid, username: &str, job: CronJobSpec) -> bool {
    let path = file_of(context);
    let began = now();
    let go = with_state(&path, |s| s.running.insert(job.id));
    if !go {
        record(
            &path,
            CronRun {
                job_id: job.id,
                tenant_id,
                started_at: began,
                status: CronStatus::Skipped,
                exit_code: None,
                output: "the last run is still going".into(),
            },
        );
        return false;
    }
    let (context, username) = (context.clone(), username.to_owned());
    tokio::spawn(async move {
        let run = execute(&context, tenant_id, &username, &job, began).await;
        with_state(&file_of(&context), |s| {
            s.running.remove(&job.id);
        });
        record(&file_of(&context), run);
    });
    true
}

async fn execute(
    context: &TenantContext,
    tenant_id: Uuid,
    username: &str,
    job: &CronJobSpec,
    started_at: i64,
) -> CronRun {
    let finish = |status, exit_code, output: String| CronRun {
        job_id: job.id,
        tenant_id,
        started_at,
        status,
        exit_code,
        output,
    };
    let container = builtpanel_domain::TenantUsername::parse(username)
        .ok()
        .and_then(|u| builtpanel_tenant_stack::php_container_name(&u).ok());
    let Some(container) = container else {
        return finish(CronStatus::Failed, None, "bad account name".into());
    };
    let result = context
        .docker
        .run_user_command(
            &container,
            &job.command,
            WORKDIR,
            Duration::from_secs(u64::from(job.timeout_seconds)),
        )
        .await;
    match result {
        Ok(done) => {
            let code = i32::try_from(done.exit_code).ok();
            // `timeout -s KILL` ends with 137 (killed) after its time; 124 is its own word.
            let status = match done.exit_code {
                0 => CronStatus::Ok,
                124 | 137 => CronStatus::Timeout,
                _ => CronStatus::Failed,
            };
            finish(status, code, done.output)
        }
        Err(builtpanel_docker_manager::DockerError::Unavailable(_)) => finish(
            CronStatus::Timeout,
            None,
            "the command did not finish in time".into(),
        ),
        Err(e) => finish(CronStatus::Failed, None, format!("could not run: {e}")),
    }
}

/// The scheduler: looks at every enabled job every few seconds, for ever.
pub async fn run_forever(context: Arc<TenantContext>) {
    let mut every = tokio::time::interval(TICK);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let path = file_of(&context);
    // A restart starts looking from now, whatever the file says.
    let started = now();
    loop {
        every.tick().await;
        let at = now();
        let due: Vec<(Uuid, String, CronJobSpec)> = with_state(&path, |s| {
            let mut due = Vec::new();
            for (tenant_id, t) in &s.tenants {
                for job in t.jobs.iter().filter(|j| j.enabled) {
                    let last = *s.looked_at.get(&job.id).unwrap_or(&started);
                    // Only the minute that has begun counts: a long pause runs a job once.
                    let hit = Cron::parse(&job.schedule)
                        .ok()
                        .and_then(|c| c.next_after(last))
                        .is_some_and(|next| next <= at);
                    if hit {
                        due.push((*tenant_id, t.username.clone(), job.clone()));
                    }
                }
            }
            for (_, _, job) in &due {
                s.looked_at.insert(job.id, at);
            }
            due
        });
        for (tenant_id, username, job) in due {
            start(&context, tenant_id, &username, job);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_is_checked_before_it_is_kept() {
        let mut job = CronJobSpec {
            id: Uuid::nil(),
            schedule: "*/5 * * * *".into(),
            command: "true".into(),
            enabled: true,
            timeout_seconds: 60,
        };
        assert!(check(&job).is_ok());
        job.schedule = "nonsense".into();
        assert!(check(&job).is_err());
        job.schedule = "0 * * * *".into();
        job.timeout_seconds = 0;
        assert!(check(&job).is_err());
        job.timeout_seconds = 60;
        job.command = "a\0b".into();
        assert!(check(&job).is_err());
    }
}
