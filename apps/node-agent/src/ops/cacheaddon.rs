//! A tenant's own cache server (T12.10): Redis or Memcached in a container of the tenant.
//!
//! The container sits on the tenant's network only (no port is published), under its container
//! name (`bp-<account>-redis`) there, runs as the tenant's own user id with a read-only root and a
//! memory limit, and is replaced when its memory or password changes. Redis keeps nothing on
//! disk and asks for a password; the password is in a small configuration file in a folder of
//! the agent's state (mounted read-only), never in the container's command line.

use std::{os::unix::fs::PermissionsExt as _, sync::Arc};

use builtpanel_docker_manager::{ContainerSpec, DockerError, Network, Restart, Tmpfs, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, network_name};

use super::{
    db,
    scan::state,
    sqlimport::failed,
    tenant::{TenantContext, docker_error, wait_healthy},
};

fn invalid(what: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, what.to_owned())
}

fn kind_of(kind: &str) -> Result<(Role, u16), AgentError> {
    match kind {
        "redis" => Ok((Role::Redis, 6379)),
        "memcached" => Ok((Role::Memcached, 11211)),
        _ => Err(invalid("the cache is redis or memcached")),
    }
}

/// The text of Redis's configuration: memory, no persistence, a password.
#[must_use]
pub fn redis_conf(memory_mb: u32, password: &str) -> String {
    format!(
        "bind 0.0.0.0\nprotected-mode yes\nport 6379\nsave \"\"\nappendonly no\n\
         maxmemory {memory_mb}mb\nmaxmemory-policy allkeys-lru\nrequirepass {password}\n\
         dir /data\n"
    )
}

/// `set_cache_instance`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); Docker or the image failing (retryable); a
/// container that stops right after starting.
pub async fn set_cache_instance(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetCacheInstance {
        instance_id,
        tenant_id: _,
        username,
        uid,
        kind,
        memory_mb,
        password,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let (role, port) = kind_of(&kind)?;
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !(1..=4096).contains(&memory_mb) || uid < 10_000 {
        return Err(invalid("the memory or the user id is not valid"));
    }
    let password = match (&kind[..], password) {
        ("redis", Some(p))
            if (12..=128).contains(&p.0.len())
                && p.0
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) =>
        {
            Some(p.0)
        }
        ("redis", _) => {
            return Err(invalid(
                "Redis needs a password of 12 to 128 letters, digits, - and _",
            ));
        }
        _ => None,
    };
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let image = allowlist
        .current(role)
        .map_err(|e| failed(&e.to_string()))?
        .clone();
    let docker = &context.docker;
    docker
        .pull_image(&image)
        .await
        .map_err(|e| docker_error(&e))?;
    let name = db::name(&format!("bp-{}-{kind}", username.as_str()))?;
    let mut spec = ContainerSpec::new(name.clone(), image);
    spec.user = Some((uid, uid));
    // The limit is for the data; the server itself needs a little more.
    spec.memory_bytes = (u64::from(memory_mb) + 48) * 1024 * 1024;
    spec.cpu_millis = 500;
    spec.pids_limit = 64;
    spec.restart = Restart::UnlessStopped;
    spec.tenant = None;
    spec.role = Some(kind.clone());
    spec.tmpfs = vec![Tmpfs {
        target: "/data".into(),
        size_bytes: 16 * 1024 * 1024,
    }];
    if let Some(password) = &password {
        let dir = state(&context)?
            .join("cache")
            .join(format!("{}-{kind}", username.as_str()));
        std::fs::create_dir_all(&dir)
            .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
        let conf = dir.join("redis.conf");
        std::fs::write(&conf, redis_conf(memory_mb, password))
            .map_err(|e| failed(&format!("cannot write the configuration: {e}")))?;
        for (path, mode) in [(&dir, 0o755), (&conf, 0o644)] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .map_err(|e| failed(&format!("cannot set rights: {e}")))?;
        }
        let volume = db::name(&format!("bp-{}-{kind}-conf", username.as_str()))?;
        let _ = docker.remove_volume(&volume).await;
        docker
            .create_bind_volume(&volume, &dir, &state(&context)?)
            .await
            .map_err(|e| docker_error(&e))?;
        spec.volumes = vec![VolumeMount {
            volume,
            target: "/opt/bp-redis".into(),
            read_only: true,
        }];
        spec.command = vec!["redis-server".into(), "/opt/bp-redis/redis.conf".into()];
    } else {
        spec.command = vec![
            "memcached".into(),
            "-m".into(),
            memory_mb.to_string(),
            "-c".into(),
            "1024".into(),
        ];
    }
    let network = network_name(&username).map_err(|e| invalid(&e.to_string()))?;
    docker
        .ensure_network(&network, false)
        .await
        .map_err(|e| docker_error(&e))?;
    // Replaced whole when it exists: a cache has nothing worth keeping across a change.
    match docker.remove(&name, true).await {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(e) => return Err(docker_error(&e)),
    }
    spec.network = Network::Named(network);
    spec.labels
        .insert("instance".into(), instance_id.simple().to_string());
    docker
        .create_container(&spec)
        .await
        .map_err(|e| docker_error(&e))?;
    docker.start(&name).await.map_err(|e| docker_error(&e))?;
    wait_healthy(docker, &name).await?;
    Ok(Output::CacheInstanceSet {
        host: name.as_str().to_owned(),
        port,
    })
}

/// `remove_cache_instance`.
///
/// # Errors
/// [`AgentError`]: bad arguments; Docker failing (retryable).
pub async fn remove_cache_instance(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RemoveCacheInstance {
        tenant_id: _,
        username,
        kind,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    kind_of(&kind)?;
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    let docker = &context.docker;
    let name = db::name(&format!("bp-{}-{kind}", username.as_str()))?;
    let existed = match docker.remove(&name, true).await {
        Ok(()) => true,
        Err(DockerError::NotFound) => false,
        Err(e) => return Err(docker_error(&e)),
    };
    let _ = docker
        .remove_volume(&db::name(&format!("bp-{}-{kind}-conf", username.as_str()))?)
        .await;
    let _ = std::fs::remove_dir_all(
        state(&context)?
            .join("cache")
            .join(format!("{}-{kind}", username.as_str())),
    );
    Ok(Output::CacheInstanceRemoved { existed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redis_configuration_has_the_password_and_the_limit() {
        let conf = redis_conf(64, "a-long-password-1");
        assert!(conf.contains("requirepass a-long-password-1") && conf.contains("maxmemory 64mb"));
        assert!(conf.contains("save \"\""));
    }
}
