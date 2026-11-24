//! `list_containers` and `restart_container` (T10.12): an account's own services.
//!
//! The account is known by the label `builtpanel.tenant` that every container of its stack
//! carries, so a name that belongs to another account (or to the node: the database, phpMyAdmin,
//! the file server) is refused, not guessed from its spelling.

use std::{sync::Arc, time::Duration};

use builtpanel_docker_manager::{DockerError, Name};
use builtpanel_node_protocol::{
    AgentError, ContainerItem, Operation, Output, ProcessItem, SignalOutcome,
};

use super::{
    sqlimport::invalid,
    tenant::{TenantContext, docker_error},
};

const GRACE: Duration = Duration::from_secs(15);

/// `list_containers`.
///
/// # Errors
/// [`AgentError`] when Docker cannot be asked (retryable).
pub async fn list_containers(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ListContainers { tenant_id } = op else {
        return Err(super::wrong_operation());
    };
    let items = context
        .docker
        .list_tenant(&tenant_id.to_string())
        .await
        .map_err(|e| docker_error(&e))?
        .into_iter()
        .map(|c| ContainerItem {
            name: c.name,
            role: c.role,
            image: c.image,
            state: c.state,
            status: c.status,
            created: c.created,
        })
        .collect();
    Ok(Output::Containers { items })
}

/// `restart_container`.
///
/// # Errors
/// [`AgentError`]: a name that is not one of the account's containers (not retryable), or
/// Docker failing.
pub async fn restart_container(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RestartContainer { tenant_id, name } = op else {
        return Err(super::wrong_operation());
    };
    let name = Name::parse(&name).map_err(|_| invalid("the container's name is not usable"))?;
    let info = context.docker.inspect(&name).await.map_err(|e| match e {
        DockerError::NotFound | DockerError::NotManaged => {
            invalid("this account has no such container")
        }
        other => docker_error(&other),
    })?;
    let mine = info
        .labels
        .get(builtpanel_docker_manager::TENANT_LABEL)
        .is_some_and(|t| *t == tenant_id.to_string());
    if !mine {
        // The same answer as for a container that is not there: nothing is given away.
        return Err(invalid("this account has no such container"));
    }
    context
        .docker
        .restart(&name, GRACE)
        .await
        .map_err(|e| docker_error(&e))?;
    let state = context
        .docker
        .inspect(&name)
        .await
        .map(|i| i.status)
        .unwrap_or_default();
    Ok(Output::ContainerRestarted {
        name: name.as_str().to_owned(),
        state,
    })
}

/// What lists the processes of a container: the shell reads `/proc` itself, so nothing has to
/// be installed in the image. Fixed text: no part of it comes from a request.
const LIST_SCRIPT: &str = r#"for p in /proc/[0-9]*; do
  pid=${p#/proc/}; uid=; st=
  [ -r "$p/status" ] || continue
  while read -r k v rest; do case "$k" in Uid:) uid=$v;; State:) st=$v;; esac; done < "$p/status"
  cmd=$(tr '\0' ' ' < "$p/cmdline" 2>/dev/null)
  [ -n "$cmd" ] || cmd="[$(cat "$p/comm" 2>/dev/null)]"
  printf '%s\t%s\t%s\t%s\n' "$pid" "$uid" "$st" "$cmd"
done"#;

fn parse_processes(container: &str, text: &str) -> Vec<ProcessItem> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.splitn(4, '\t');
            let pid = f.next()?.parse().ok()?;
            let uid = f.next()?.parse().ok()?;
            let state = f.next()?.to_owned();
            let command: String = f.next()?.trim().chars().take(300).collect();
            Some(ProcessItem {
                container: container.to_owned(),
                pid,
                uid,
                state,
                command,
            })
        })
        .collect()
}

/// `list_processes`.
///
/// # Errors
/// [`AgentError`] when Docker cannot be asked (retryable).
pub async fn list_processes(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ListProcesses { tenant_id } = op else {
        return Err(super::wrong_operation());
    };
    let mut items = Vec::new();
    for container in context
        .docker
        .list_tenant(&tenant_id.to_string())
        .await
        .map_err(|e| docker_error(&e))?
        .into_iter()
        .filter(|c| c.state == "running")
    {
        let Ok(name) = Name::parse(&container.name) else {
            continue;
        };
        match context
            .docker
            .run_user_command(&name, LIST_SCRIPT, "/", Duration::from_secs(10))
            .await
        {
            Ok(done) if done.exit_code == 0 => {
                items.extend(parse_processes(&container.name, &done.output));
            }
            // A container that stopped meanwhile, or one that cannot run a shell: no list.
            _ => {}
        }
    }
    items.truncate(2000);
    Ok(Output::Processes { items })
}

/// `signal_process`.
///
/// # Errors
/// [`AgentError`]: a container that is not the account's, or process 1 or 0 (not retryable);
/// Docker failing.
pub async fn signal_process(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SignalProcess {
        tenant_id,
        container,
        pid,
        kill,
        escalate,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if pid < 2 {
        return Err(invalid(
            "process 1 cannot be signalled: restart the service instead",
        ));
    }
    let name =
        Name::parse(&container).map_err(|_| invalid("the container's name is not usable"))?;
    let info = context.docker.inspect(&name).await.map_err(|e| match e {
        DockerError::NotFound | DockerError::NotManaged => {
            invalid("this account has no such container")
        }
        other => docker_error(&other),
    })?;
    if info
        .labels
        .get(builtpanel_docker_manager::TENANT_LABEL)
        .is_none_or(|t| *t != tenant_id.to_string())
    {
        return Err(invalid("this account has no such container"));
    }
    // `pid` is a number and the signal one of two words: the only parts of the command.
    let run = |command: String| {
        let (docker, name) = (context.docker.clone(), name.clone());
        async move {
            docker
                .run_user_command(&name, &command, "/", Duration::from_secs(10))
                .await
        }
    };
    // Alive: signalling it is possible and it is not a zombie (an ended process that its parent
    // has not collected yet counts as gone; `kill -0` alone would say it is still there).
    let alive = |pid: u32| async move {
        run(format!(
            "kill -0 {pid} 2>/dev/null && {{ while read -r k v r; do [ \"$k\" = State: ] && [ \"$v\" = Z ] && exit 1; done < /proc/{pid}/status; exit 0; }}"
        ))
        .await
        .is_ok_and(|d| d.exit_code == 0)
    };
    if !alive(pid).await {
        return Ok(Output::ProcessSignalled {
            pid,
            outcome: SignalOutcome::NotFound,
        });
    }
    let signal = if kill { "KILL" } else { "TERM" };
    run(format!("kill -s {signal} {pid} 2>/dev/null"))
        .await
        .map_err(|e| docker_error(&e))?;
    if kill {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let outcome = if alive(pid).await {
            SignalOutcome::StillRunning
        } else {
            SignalOutcome::Killed
        };
        return Ok(Output::ProcessSignalled { pid, outcome });
    }
    // SIGTERM: give it time; with `escalate`, kill what stays.
    let waited = if escalate { 10 } else { 2 };
    for _ in 0..waited * 4 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if !alive(pid).await {
            return Ok(Output::ProcessSignalled {
                pid,
                outcome: SignalOutcome::Exited,
            });
        }
    }
    if escalate {
        run(format!("kill -s KILL {pid} 2>/dev/null"))
            .await
            .map_err(|e| docker_error(&e))?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let outcome = if alive(pid).await {
            SignalOutcome::StillRunning
        } else {
            SignalOutcome::Killed
        };
        return Ok(Output::ProcessSignalled { pid, outcome });
    }
    Ok(Output::ProcessSignalled {
        pid,
        outcome: SignalOutcome::StillRunning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_is_read_line_by_line_and_junk_is_left_out() {
        let text = "1\t10001\tS (sleeping)\tsleep 600\n17\t10001\tR (running)\tphp-fpm: pool www \nnot a line\n18\tx\tS\tbad uid\n";
        let found = parse_processes("bp-a-php", text);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[1].pid, found[1].command.as_str()),
            (17, "php-fpm: pool www")
        );
    }
}
