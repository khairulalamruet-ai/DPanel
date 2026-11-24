//! A tenant's databases moving between the node's shared database server and the tenant's own
//! (T07.13), and the tenant's own server going away.
//!
//! One operation does it all on the node, in an order that can be repeated: the target server
//! is made, the users are made there with the passwords they have (the hashes are copied from
//! one server to the other on the node; no password is sent or kept), each database is dumped
//! from the source (as the temporary read-only account an export uses) and loaded into the
//! target (a copy left by an earlier try is dropped first), and the grants and remote addresses
//! are set again. The source stays as it was until the control plane, having switched, asks for
//! it to be removed.

use std::sync::Arc;

use builtpanel_docker_manager::DockerError;
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{
    AgentError, DbEngine, ErrorCode, MAX_MIGRATED_DATABASES, Operation, Output,
};
use uuid::Uuid;

use super::{
    db, dbserver, pg, sqlexport, sqlimport,
    tenant::{TenantContext, docker_error},
};

fn invalid(message: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, message.to_owned())
}

fn scope_of(tenant: &str, dedicated: bool) -> Option<String> {
    dedicated.then(|| tenant.to_owned())
}

/// `migrate_tenant_databases`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); a server failing (retryable when it is not
/// reachable).
pub async fn migrate(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::MigrateTenantDatabases {
        username,
        engine,
        to_dedicated,
        databases,
        users,
        grants,
        remote_hosts,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if databases.len() > MAX_MIGRATED_DATABASES || users.len() > MAX_MIGRATED_DATABASES {
        return Err(invalid("too many databases or users"));
    }
    for name in databases.iter().chain(users.iter()) {
        builtpanel_domain::databases::check_identifier(name)
            .map_err(|e| invalid(&e.to_string()))?;
        if engine == DbEngine::Postgres {
            pg::identifier(name)?;
        }
    }
    let source = scope_of(&username, !to_dedicated);
    let target = scope_of(&username, to_dedicated);

    // 1. The target server, and the users on it with the passwords they have.
    for user in &users {
        let hash = dbserver::scope(
            source.clone(),
            read_credential(&context, engine, &tenant, user),
        )
        .await?
        .ok_or_else(|| invalid("a database user to move does not exist on the source"))?;
        dbserver::scope(
            target.clone(),
            write_credential(&context, engine, &tenant, user, &hash),
        )
        .await?;
    }
    if users.is_empty() {
        dbserver::scope(target.clone(), ensure(&context, engine)).await?;
    }

    // 2. The databases: the target's copy is made new each time.
    let dir = sqlimport::imports_dir(&context);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AgentError::new(
            ErrorCode::Internal,
            format!("cannot prepare the import directory: {}", e.kind()),
        )
        .retryable()
    })?;
    for database in &databases {
        let id = Uuid::now_v7();
        let path = dir.join(format!("{}.move", id.simple()));
        let copied = async {
            dbserver::scope(
                source.clone(),
                sqlexport::dump_to_file(
                    &context,
                    engine,
                    database,
                    &id.simple().to_string()[..12],
                    &path,
                ),
            )
            .await?;
            dbserver::scope(target.clone(), load(&context, engine, database, id, &path)).await
        }
        .await;
        let _ = std::fs::remove_file(&path);
        copied?;
    }

    // 3. Who may do what, and from where.
    for g in &grants {
        let op = Operation::GrantDb {
            database: g.database.clone(),
            user: g.user.clone(),
            access: g.access.clone(),
            engine,
            dedicated: target.clone(),
        };
        dbserver::scope(target.clone(), async {
            match engine {
                DbEngine::Mysql => db::grant_db(context.clone(), op).await,
                DbEngine::Postgres => pg::grant_db(context.clone(), op).await,
            }
        })
        .await?;
    }
    for r in &remote_hosts {
        let op = Operation::SetDbRemoteHosts {
            username: username.clone(),
            name: r.user.clone(),
            cidrs: r.cidrs.clone(),
            engine,
            dedicated: target.clone(),
        };
        dbserver::scope(target.clone(), async {
            match engine {
                DbEngine::Mysql => db::set_db_remote_hosts(context.clone(), op).await,
                DbEngine::Postgres => pg::set_db_remote_hosts(context.clone(), op).await,
            }
        })
        .await?;
    }
    // The tenant's containers now find the target as `db` (or `pg`), and not the source: the
    // source is taken off the tenant's network.
    let network =
        builtpanel_tenant_stack::network_name(&tenant).map_err(|e| invalid(&e.to_string()))?;
    let source_container = dbserver::scope(source.clone(), async {
        match engine {
            DbEngine::Mysql => db::container_name(),
            DbEngine::Postgres => pg::container_name(),
        }
    })
    .await;
    match context
        .docker
        .disconnect_network(&db::name(&source_container)?, &network)
        .await
    {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(e) => return Err(docker_error(&e)),
    }
    Ok(Output::DbMigrated {
        databases: databases.len(),
        users: users.len(),
    })
}

async fn ensure(context: &TenantContext, engine: DbEngine) -> Result<(), AgentError> {
    match engine {
        DbEngine::Mysql => db::ensure_server(context).await.map(|_| ()),
        DbEngine::Postgres => pg::ensure_server(context).await.map(|_| ()),
    }
}

/// A user's password as the source server holds it.
async fn read_credential(
    context: &TenantContext,
    engine: DbEngine,
    tenant: &TenantUsername,
    user: &str,
) -> Result<Option<String>, AgentError> {
    match engine {
        DbEngine::Mysql => db::user_hash(context, tenant, user).await,
        DbEngine::Postgres => pg::user_verifier(context, user).await,
    }
}

async fn write_credential(
    context: &TenantContext,
    engine: DbEngine,
    tenant: &TenantUsername,
    user: &str,
    credential: &str,
) -> Result<(), AgentError> {
    match engine {
        DbEngine::Mysql => db::set_user_hash(context, tenant, user, credential).await,
        DbEngine::Postgres => pg::set_user_verifier(context, tenant, user, credential).await,
    }
}

/// Makes the database on the target new and loads the dump into it.
async fn load(
    context: &Arc<TenantContext>,
    engine: DbEngine,
    database: &str,
    id: Uuid,
    path: &std::path::Path,
) -> Result<(), AgentError> {
    let drop_op = Operation::DropDatabase {
        name: database.to_owned(),
        engine,
        dedicated: dbserver::current(),
    };
    let create_op = Operation::CreateDatabase {
        name: database.to_owned(),
        engine,
        dedicated: dbserver::current(),
    };
    match engine {
        DbEngine::Mysql => {
            db::drop_database(context.clone(), drop_op).await?;
            db::create_database(context.clone(), create_op).await?;
            sqlimport::import_file(context, database, id, path, true).await?;
        }
        DbEngine::Postgres => {
            pg::drop_database(context.clone(), drop_op).await?;
            pg::create_database(context.clone(), create_op).await?;
            super::pgimport::import_file(context, database, id, path, true).await?;
        }
    }
    Ok(())
}

/// `remove_dedicated_db`.
///
/// # Errors
/// [`AgentError`]: a bad name (not retryable), Docker failing (retryable).
pub async fn remove(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::RemoveDedicatedDb { username, engine } = op else {
        return Err(super::wrong_operation());
    };
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    dbserver::scope(Some(username.clone()), async {
        let docker = &context.docker;
        let (container, volume, networks, kind) = match engine {
            DbEngine::Mysql => (
                db::container_name(),
                dbserver::volume(dbserver::Kind::Mariadb, "bp-db-data"),
                vec![format!("bp-{username}-dbnet")],
                dbserver::Kind::Mariadb,
            ),
            DbEngine::Postgres => (
                pg::container_name(),
                dbserver::volume(dbserver::Kind::Postgres, "bp-pg-data"),
                vec![format!("bp-{username}-pgnet")],
                dbserver::Kind::Postgres,
            ),
        };
        let name = |text: &str| db::name(text);
        let existed = match docker.inspect(&name(&container)?).await {
            Ok(_) => {
                match docker
                    .stop(&name(&container)?, std::time::Duration::from_secs(30))
                    .await
                {
                    Ok(()) | Err(DockerError::NotFound) => {}
                    Err(e) => return Err(docker_error(&e)),
                }
                match docker.remove(&name(&container)?, true).await {
                    Ok(()) | Err(DockerError::NotFound) => {}
                    Err(e) => return Err(docker_error(&e)),
                }
                true
            }
            Err(DockerError::NotFound) => false,
            Err(e) => return Err(docker_error(&e)),
        };
        let mut volumes = vec![volume];
        if engine == DbEngine::Postgres {
            volumes.push(format!("bp-{username}-pg-conf"));
        }
        for v in volumes {
            match docker.remove_volume(&name(&v)?).await {
                Ok(()) | Err(DockerError::NotFound) => {}
                Err(e) => return Err(docker_error(&e)),
            }
        }
        for n in networks {
            match docker.remove_network(&name(&n)?).await {
                Ok(()) | Err(DockerError::NotFound) => {}
                Err(e) => return Err(docker_error(&e)),
            }
        }
        let _ =
            std::fs::remove_file(dbserver::secret_dir(&context).join(dbserver::secret_name(kind)));
        if engine == DbEngine::Postgres {
            pg::forget_state(&context);
        }
        dbserver::forget_port(&context, kind, &username);
        let _ = tenant;
        Ok(Output::DedicatedDbRemoved { existed })
    })
    .await
}

/// Removes both of a tenant's own database servers (the tenant is being deleted).
///
/// # Errors
/// Docker failing (retryable).
pub async fn remove_all(context: &Arc<TenantContext>, username: &str) -> Result<(), AgentError> {
    for engine in [DbEngine::Mysql, DbEngine::Postgres] {
        remove(
            context.clone(),
            Operation::RemoveDedicatedDb {
                username: username.to_owned(),
                engine,
            },
        )
        .await?;
    }
    Ok(())
}
