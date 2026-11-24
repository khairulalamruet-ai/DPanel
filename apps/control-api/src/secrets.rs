//! Secrets that are kept out of the operations ledger and put back when an
//! operation is sent to its node (T05.17b).

use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_dispatcher::SecretFiller;
use builtpanel_node_protocol::Operation;

/// Fills in the private keys of uploaded certificates: the ledger has the chain
/// only, the sealed key is opened here, for the one frame that carries it. A
/// certificate that was removed since the operation was queued is left out (the
/// removal has already queued the next operation).
pub fn operation_secrets(
    db: builtpanel_database::sqlx::PgPool,
    master_key: MasterKey,
) -> SecretFiller {
    std::sync::Arc::new(move |op| {
        let (db, master_key) = (db.clone(), master_key.clone());
        Box::pin(async move {
            let (username, mut items) = match op {
                Operation::SetCustomCertificates { username, items } => (username, items),
                // A database user's password: sealed by the API, opened here for this
                // one message. Without one (it was delivered already) the node keeps
                // the password the user has.
                Operation::UpsertDbUser {
                    username,
                    name,
                    password: None,
                    engine,
                    dedicated,
                } => {
                    let found = builtpanel_database::databases::sealed_password_of(&db, &name)
                        .await
                        .map_err(|_| "could not read a database password".to_owned())?;
                    let password = match found {
                        Some((user_id, sealed)) => {
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::databases::password_context(user_id),
                                    &sealed,
                                )
                                .map_err(|_| "a database password does not open".to_owned())?;
                            Some(builtpanel_node_protocol::DbPassword(
                                String::from_utf8(bytes)
                                    .map_err(|_| "a database password is not text".to_owned())?,
                            ))
                        }
                        None => None,
                    };
                    return Ok(Operation::UpsertDbUser {
                        username,
                        name,
                        password,
                        engine,
                        dedicated,
                    });
                }
                // A mailbox's password: sealed by the API, opened here for this one message.
                // Without one (it was delivered already) the mail server keeps the password
                // the mailbox has.
                Operation::UpsertMailbox {
                    domain,
                    local,
                    password: None,
                    quota_bytes,
                    aliases,
                } => {
                    let found = builtpanel_database::mail::sealed_password_of(&db, &domain, &local)
                        .await
                        .map_err(|_| "could not read a mailbox password".to_owned())?;
                    let password = match found {
                        Some((mailbox_id, sealed)) => {
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::mail::password_context(mailbox_id),
                                    &sealed,
                                )
                                .map_err(|_| "a mailbox password does not open".to_owned())?;
                            Some(builtpanel_node_protocol::DbPassword(
                                String::from_utf8(bytes)
                                    .map_err(|_| "a mailbox password is not text".to_owned())?,
                            ))
                        }
                        None => None,
                    };
                    return Ok(Operation::UpsertMailbox {
                        domain,
                        local,
                        password,
                        quota_bytes,
                        aliases,
                    });
                }
                // The two passwords of a WordPress install: sealed by the API, opened here
                // for this one message.
                Operation::InstallWordPress {
                    app_id,
                    tenant_id,
                    username,
                    uid,
                    path,
                    database,
                    database_password: None,
                    site,
                    admin_password: None,
                    version,
                    locale,
                    plugins,
                } => {
                    let open = |name: &'static str| {
                        let (db, master_key) = (db.clone(), master_key.clone());
                        async move {
                            let sealed =
                                builtpanel_database::apps::sealed_secret(&db, app_id, name)
                                    .await
                                    .map_err(|_| "could not read an install secret".to_owned())?
                                    .ok_or_else(|| "this install has ended".to_owned())?;
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::apps::secret_context(app_id, name),
                                    &sealed,
                                )
                                .map_err(|_| "an install secret does not open".to_owned())?;
                            String::from_utf8(bytes)
                                .map_err(|_| "an install secret is not text".to_owned())
                                .map(builtpanel_node_protocol::DbPassword)
                        }
                    };
                    return Ok(Operation::InstallWordPress {
                        app_id,
                        tenant_id,
                        username,
                        uid,
                        path,
                        database,
                        database_password: Some(open("db_password").await?),
                        site,
                        admin_password: Some(open("admin_password").await?),
                        version,
                        locale,
                        plugins,
                    });
                }
                // The same for an application of the catalog: the database user's password
                // when it has a database, the administrator's when it has one.
                Operation::InstallApp {
                    app_id,
                    tenant_id,
                    username,
                    uid,
                    path,
                    definition,
                    database,
                    database_password: None,
                    site,
                    admin_password: None,
                } => {
                    let open = |name: &'static str| {
                        let (db, master_key) = (db.clone(), master_key.clone());
                        async move {
                            let sealed =
                                builtpanel_database::apps::sealed_secret(&db, app_id, name)
                                    .await
                                    .map_err(|_| "could not read an install secret".to_owned())?
                                    .ok_or_else(|| "this install has ended".to_owned())?;
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::apps::secret_context(app_id, name),
                                    &sealed,
                                )
                                .map_err(|_| "an install secret does not open".to_owned())?;
                            String::from_utf8(bytes)
                                .map_err(|_| "an install secret is not text".to_owned())
                                .map(builtpanel_node_protocol::DbPassword)
                        }
                    };
                    let database_password = if database.is_some() {
                        Some(open("db_password").await?)
                    } else {
                        None
                    };
                    let admin_password = if definition.needs_admin() {
                        Some(open("admin_password").await?)
                    } else {
                        None
                    };
                    return Ok(Operation::InstallApp {
                        app_id,
                        tenant_id,
                        username,
                        uid,
                        path,
                        definition,
                        database,
                        database_password,
                        site,
                        admin_password,
                    });
                }
                // A cache server's password: sealed by the API, opened here for this one
                // message.
                Operation::SetCacheInstance {
                    instance_id,
                    tenant_id,
                    username,
                    uid,
                    kind,
                    memory_mb,
                    password: None,
                } => {
                    let password = if kind == "redis" {
                        let sealed =
                            builtpanel_database::cache_instances::sealed_password(&db, instance_id)
                                .await
                                .map_err(|_| "could not read a cache password".to_owned())?
                                .ok_or_else(|| "this cache server has no password".to_owned())?;
                        let bytes = master_key
                            .open(
                                &builtpanel_database::cache_instances::password_context(
                                    instance_id,
                                ),
                                &sealed,
                            )
                            .map_err(|_| "a cache password does not open".to_owned())?;
                        Some(builtpanel_node_protocol::DbPassword(
                            String::from_utf8(bytes)
                                .map_err(|_| "a cache password is not text".to_owned())?,
                        ))
                    } else {
                        None
                    };
                    return Ok(Operation::SetCacheInstance {
                        instance_id,
                        tenant_id,
                        username,
                        uid,
                        kind,
                        memory_mb,
                        password,
                    });
                }
                // The token for a private repository: sealed by the API, opened here for
                // this one message.
                Operation::RunGitDeploy {
                    deploy_id,
                    tenant_id,
                    username,
                    uid,
                    name,
                    repo_url,
                    branch,
                    dest,
                    build_kind,
                    build_version,
                    build_steps,
                    restart_app,
                    token: None,
                } => {
                    let sealed = builtpanel_database::git_deploys::sealed_token(&db, deploy_id)
                        .await
                        .map_err(|_| "could not read a deploy's token".to_owned())?;
                    let token = match sealed {
                        None => None,
                        Some(sealed) => {
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::git_deploys::token_context(deploy_id),
                                    &sealed,
                                )
                                .map_err(|_| "a deploy's token does not open".to_owned())?;
                            Some(builtpanel_node_protocol::DbPassword(
                                String::from_utf8(bytes)
                                    .map_err(|_| "a deploy's token is not text".to_owned())?,
                            ))
                        }
                    };
                    return Ok(Operation::RunGitDeploy {
                        deploy_id,
                        tenant_id,
                        username,
                        uid,
                        name,
                        repo_url,
                        branch,
                        dest,
                        build_kind,
                        build_version,
                        build_steps,
                        restart_app,
                        token,
                    });
                }
                // An application's environment: sealed by the API, opened here for this one
                // message.
                Operation::SetRuntimeApp {
                    app_id,
                    tenant_id,
                    username,
                    uid,
                    kind,
                    version,
                    name,
                    dir,
                    command,
                    install,
                    port,
                    memory_mb,
                    env: None,
                    image,
                } => {
                    let sealed = builtpanel_database::runtime_apps::sealed_env(&db, app_id)
                        .await
                        .map_err(|_| "could not read an application's environment".to_owned())?;
                    let env = match sealed {
                        None => Vec::new(),
                        Some(sealed) => {
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::runtime_apps::env_context(app_id),
                                    &sealed,
                                )
                                .map_err(|_| {
                                    "an application's environment does not open".to_owned()
                                })?;
                            serde_json::from_slice::<std::collections::BTreeMap<String, String>>(
                                &bytes,
                            )
                            .map_err(|_| "an application's environment is not readable".to_owned())?
                            .into_iter()
                            .collect()
                        }
                    };
                    return Ok(Operation::SetRuntimeApp {
                        app_id,
                        tenant_id,
                        username,
                        uid,
                        kind,
                        version,
                        name,
                        dir,
                        command,
                        install,
                        port,
                        memory_mb,
                        env: Some(builtpanel_node_protocol::RuntimeEnv(env)),
                        image,
                    });
                }
                // A WordPress user's new password: sealed by the API, opened here for this
                // one message.
                Operation::RunWpCli {
                    app_id,
                    tenant_id,
                    username,
                    uid,
                    path,
                    command:
                        command @ builtpanel_node_protocol::WpCommand::ResetAdminPassword { .. },
                    password: None,
                } => {
                    let sealed =
                        builtpanel_database::apps::sealed_secret(&db, app_id, "admin_reset")
                            .await
                            .map_err(|_| "could not read a password".to_owned())?
                            .ok_or_else(|| "this password was used already".to_owned())?;
                    let bytes = master_key
                        .open(
                            &builtpanel_database::apps::secret_context(app_id, "admin_reset"),
                            &sealed,
                        )
                        .map_err(|_| "a password does not open".to_owned())?;
                    return Ok(Operation::RunWpCli {
                        app_id,
                        tenant_id,
                        username,
                        uid,
                        path,
                        command,
                        password: Some(builtpanel_node_protocol::DbPassword(
                            String::from_utf8(bytes)
                                .map_err(|_| "a password is not text".to_owned())?,
                        )),
                    });
                }
                // The download credential of an SQL import: sealed by the API, opened
                // for this one message. Once the import has ended there is none, and
                // the node's download (which would be refused anyway) is not tried.
                Operation::ImportSql {
                    username,
                    database,
                    import_id,
                    url,
                    size_bytes,
                    sha256,
                    gzip,
                    token: None,
                    engine,
                    dedicated,
                } => {
                    let sealed = builtpanel_database::imports::sealed_token_of(&db, import_id)
                        .await
                        .map_err(|_| "could not read an import token".to_owned())?
                        .ok_or_else(|| "this import has ended".to_owned())?;
                    let bytes = master_key
                        .open(
                            &builtpanel_database::imports::token_context(import_id),
                            &sealed,
                        )
                        .map_err(|_| "an import token does not open".to_owned())?;
                    return Ok(Operation::ImportSql {
                        username,
                        database,
                        import_id,
                        url,
                        size_bytes,
                        sha256,
                        gzip,
                        token: Some(builtpanel_node_protocol::DbPassword(
                            String::from_utf8(bytes)
                                .map_err(|_| "an import token is not text".to_owned())?,
                        )),
                        engine,
                        dedicated,
                    });
                }
                // The same for the upload credential of an SQL export.
                Operation::ExportSql {
                    username,
                    database,
                    export_id,
                    url,
                    token: None,
                    engine,
                    dedicated,
                } => {
                    let sealed = builtpanel_database::exports::sealed_token_of(&db, export_id)
                        .await
                        .map_err(|_| "could not read an export token".to_owned())?
                        .ok_or_else(|| "this export has ended".to_owned())?;
                    let bytes = master_key
                        .open(
                            &builtpanel_database::exports::token_context(export_id),
                            &sealed,
                        )
                        .map_err(|_| "an export token does not open".to_owned())?;
                    return Ok(Operation::ExportSql {
                        username,
                        database,
                        export_id,
                        url,
                        token: Some(builtpanel_node_protocol::DbPassword(
                            String::from_utf8(bytes)
                                .map_err(|_| "an export token is not text".to_owned())?,
                        )),
                        engine,
                        dedicated,
                    });
                }
                // An FTP account's password: sealed by the API, opened here for this one
                // message. Without one (it was delivered already) the account keeps it.
                Operation::UpsertFtpAccount {
                    tenant_id,
                    uid,
                    name,
                    home,
                    password: None,
                    sftp,
                    ftps,
                    ftp_plain,
                    read_only,
                    quota_mb,
                    enabled,
                } => {
                    let found = builtpanel_database::ftp_accounts::sealed_password_of(&db, &name)
                        .await
                        .map_err(|_| "could not read an FTP password".to_owned())?;
                    let password = match found {
                        Some((account_id, sealed)) => {
                            let bytes = master_key
                                .open(
                                    &builtpanel_database::ftp_accounts::password_context(
                                        account_id,
                                    ),
                                    &sealed,
                                )
                                .map_err(|_| "an FTP password does not open".to_owned())?;
                            Some(builtpanel_node_protocol::DbPassword(
                                String::from_utf8(bytes)
                                    .map_err(|_| "an FTP password is not text".to_owned())?,
                            ))
                        }
                        None => None,
                    };
                    return Ok(Operation::UpsertFtpAccount {
                        tenant_id,
                        uid,
                        name,
                        home,
                        password,
                        sftp,
                        ftps,
                        ftp_plain,
                        read_only,
                        quota_mb,
                        enabled,
                    });
                }
                // The credential of a big file transfer, sealed by the API and opened
                // for this one message.
                Operation::FetchFile {
                    tenant_id,
                    uid,
                    transfer_id,
                    dest,
                    url,
                    size_bytes,
                    overwrite,
                    token: None,
                } => {
                    let token = transfer_token(&db, &master_key, transfer_id).await?;
                    return Ok(Operation::FetchFile {
                        tenant_id,
                        uid,
                        transfer_id,
                        dest,
                        url,
                        size_bytes,
                        overwrite,
                        token: Some(token),
                    });
                }
                Operation::SendFile {
                    tenant_id,
                    uid,
                    transfer_id,
                    path,
                    url,
                    max_bytes,
                    token: None,
                } => {
                    let token = transfer_token(&db, &master_key, transfer_id).await?;
                    return Ok(Operation::SendFile {
                        tenant_id,
                        uid,
                        transfer_id,
                        path,
                        url,
                        max_bytes,
                        token: Some(token),
                    });
                }
                // The key and the destination's credentials of a backup.
                op @ (Operation::CreateBackup { .. }
                | Operation::DeleteBackup { .. }
                | Operation::RestoreBackup { .. }
                | Operation::ExportBackup { .. }
                | Operation::ImportExport { .. }) => {
                    return builtpanel_mod_backups::runner::fill(&db, &master_key, op).await;
                }
                other => return Ok(other),
            };
            let mut filled = Vec::with_capacity(items.len());
            for mut item in items.drain(..) {
                if item.private_key.is_none() {
                    let found = builtpanel_database::certificates::custom_key_of(
                        &db, &username, &item.host,
                    )
                    .await
                    .map_err(|_| "could not read a certificate key".to_owned())?;
                    let Some((domain_id, sealed)) = found else {
                        continue;
                    };
                    let key = master_key
                        .open(
                            &builtpanel_database::certificates::key_context(domain_id),
                            &sealed,
                        )
                        .map_err(|_| "a certificate key does not open".to_owned())?;
                    item.private_key = Some(
                        String::from_utf8(key)
                            .map_err(|_| "a certificate key is not text".to_owned())?,
                    );
                }
                filled.push(item);
            }
            Ok(Operation::SetCustomCertificates {
                username,
                items: filled,
            })
        })
    })
}

/// The opened token of a file transfer that is still wanted.
async fn transfer_token(
    db: &builtpanel_database::sqlx::PgPool,
    master_key: &MasterKey,
    transfer_id: builtpanel_database::sqlx::types::Uuid,
) -> Result<builtpanel_node_protocol::DbPassword, String> {
    let sealed = builtpanel_database::file_transfers::sealed_token_of(db, transfer_id)
        .await
        .map_err(|_| "could not read a transfer token".to_owned())?
        .ok_or_else(|| "this transfer has ended".to_owned())?;
    let bytes = master_key
        .open(
            &builtpanel_database::file_transfers::token_context(transfer_id),
            &sealed,
        )
        .map_err(|_| "a transfer token does not open".to_owned())?;
    Ok(builtpanel_node_protocol::DbPassword(
        String::from_utf8(bytes).map_err(|_| "a transfer token is not text".to_owned())?,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use builtpanel_node_protocol::RuntimeEnv;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The environment of an application is not in the ledger, and is opened for the one
    /// message that carries it.
    #[sqlx::test(migrations = "../../migrations")]
    #[ignore = "needs PostgreSQL"]
    async fn an_applications_environment_is_put_back_when_the_operation_is_sent(pool: PgPool) {
        let key = MasterKey::from_bytes(&[7u8; 32]);
        let account: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('client', 'gus', 'gus@example.com',
                     '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA')
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let node: Uuid =
            sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node1') RETURNING id")
                .fetch_one(&pool)
                .await
                .unwrap();
        let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
            .fetch_one(&pool)
            .await
            .unwrap();
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
             VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
        )
        .bind(account)
        .bind(node)
        .bind(plan)
        .fetch_one(&pool)
        .await
        .unwrap();
        let app: Uuid = sqlx::query_scalar(
            "INSERT INTO runtime_apps (tenant_id, name, kind, version, start_command)
             VALUES ($1, 'api', 'node', '24', 'node a.js') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
        let sealed = key
            .seal(
                &builtpanel_database::runtime_apps::env_context(app),
                br#"{"A":"1","SECRET":"it's s3cret"}"#,
            )
            .unwrap();
        sqlx::query("UPDATE runtime_apps SET sealed_env = $2 WHERE id = $1")
            .bind(app)
            .bind(&sealed)
            .execute(&pool)
            .await
            .unwrap();
        let op = Operation::SetRuntimeApp {
            app_id: app,
            tenant_id: tenant,
            username: "gus".into(),
            uid: 10_007,
            kind: "node".into(),
            version: "24".into(),
            name: "api".into(),
            dir: String::new(),
            command: vec!["node".into(), "a.js".into()],
            install: false,
            port: 3000,
            memory_mb: 256,
            env: None,
            image: None,
        };
        let fill = operation_secrets(pool.clone(), key.clone());
        let Operation::SetRuntimeApp { env, .. } = fill(op.clone()).await.unwrap() else {
            panic!("the same kind of operation");
        };
        assert_eq!(
            env,
            Some(RuntimeEnv(vec![
                ("A".into(), "1".into()),
                ("SECRET".into(), "it's s3cret".into())
            ]))
        );
        // Another application's sealed value does not open under this one's context.
        let other = Uuid::now_v7();
        sqlx::query("UPDATE runtime_apps SET sealed_env = $2 WHERE id = $1")
            .bind(app)
            .bind(
                key.seal(
                    &builtpanel_database::runtime_apps::env_context(other),
                    b"{}",
                )
                .unwrap(),
            )
            .execute(&pool)
            .await
            .unwrap();
        assert!(fill(op).await.is_err());
    }

    /// A mailbox's password is not in the ledger and is opened for the one message; once the
    /// node has it (the row is gone) the operation goes without one.
    #[sqlx::test(migrations = "../../migrations")]
    #[ignore = "needs PostgreSQL"]
    async fn a_mailboxs_password_is_put_back_when_the_operation_is_sent(pool: PgPool) {
        let key = MasterKey::from_bytes(&[5u8; 32]);
        let account: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('client', 'gus', 'gus@example.com',
                     '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA')
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let node: Uuid =
            sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node1') RETURNING id")
                .fetch_one(&pool)
                .await
                .unwrap();
        let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
            .fetch_one(&pool)
            .await
            .unwrap();
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
             VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
        )
        .bind(account)
        .bind(node)
        .bind(plan)
        .fetch_one(&pool)
        .await
        .unwrap();
        let domain: Uuid = sqlx::query_scalar(
            "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'gus-mail.org', 'primary')
             RETURNING id",
        )
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
        let md = builtpanel_database::mail::ensure_domain(
            &pool,
            tenant,
            domain,
            "gus-mail.org",
            node,
            None,
        )
        .await
        .unwrap();
        let mailbox = Uuid::new_v4();
        let sealed = key
            .seal(
                &builtpanel_database::mail::password_context(mailbox),
                b"Mailbox-Pass-77!",
            )
            .unwrap();
        builtpanel_database::mail::create_mailbox(
            &pool, mailbox, tenant, md.id, "gus", 100, &sealed,
        )
        .await
        .unwrap();
        let op = Operation::UpsertMailbox {
            domain: "gus-mail.org".into(),
            local: "gus".into(),
            password: None,
            quota_bytes: 100 << 20,
            aliases: Vec::new(),
        };
        let fill = operation_secrets(pool.clone(), key.clone());
        let Operation::UpsertMailbox { password, .. } = fill(op.clone()).await.unwrap() else {
            panic!("the same kind of operation");
        };
        assert_eq!(
            password,
            Some(builtpanel_node_protocol::DbPassword(
                "Mailbox-Pass-77!".into()
            ))
        );
        // The node has it: nothing is put in any more.
        builtpanel_database::mail::mailbox_applied(&pool, mailbox, 1)
            .await
            .unwrap();
        let Operation::UpsertMailbox { password, .. } = fill(op).await.unwrap() else {
            panic!("the same kind of operation");
        };
        assert_eq!(password, None);
    }

    /// The token of a private repository is not in the ledger and is opened for the one message.
    #[sqlx::test(migrations = "../../migrations")]
    #[ignore = "needs PostgreSQL"]
    async fn a_deploys_token_is_put_back_when_the_operation_is_sent(pool: PgPool) {
        let key = MasterKey::from_bytes(&[9u8; 32]);
        let account: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('client', 'gus', 'gus@example.com',
                     '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA')
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let node: Uuid =
            sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node1') RETURNING id")
                .fetch_one(&pool)
                .await
                .unwrap();
        let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
            .fetch_one(&pool)
            .await
            .unwrap();
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
             VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
        )
        .bind(account)
        .bind(node)
        .bind(plan)
        .fetch_one(&pool)
        .await
        .unwrap();
        let deploy: Uuid = sqlx::query_scalar(
            "INSERT INTO git_deploys (tenant_id, name, repo_url, dest, sealed_webhook)
             VALUES ($1, 'site', 'https://git.example.com/a/b.git', 'site', '\\x00') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
        let sealed = key
            .seal(
                &builtpanel_database::git_deploys::token_context(deploy),
                b"ghp_tokenvalue",
            )
            .unwrap();
        sqlx::query("UPDATE git_deploys SET sealed_token = $2 WHERE id = $1")
            .bind(deploy)
            .bind(&sealed)
            .execute(&pool)
            .await
            .unwrap();
        let op = Operation::RunGitDeploy {
            deploy_id: deploy,
            tenant_id: tenant,
            username: "gus".into(),
            uid: 10_007,
            name: "site".into(),
            repo_url: "https://git.example.com/a/b.git".into(),
            branch: "main".into(),
            dest: "site".into(),
            build_kind: None,
            build_version: None,
            build_steps: Vec::new(),
            restart_app: None,
            token: None,
        };
        let Operation::RunGitDeploy { token, .. } =
            operation_secrets(pool.clone(), key.clone())(op.clone())
                .await
                .unwrap()
        else {
            panic!("the same kind of operation");
        };
        assert_eq!(token.map(|t| t.0), Some("ghp_tokenvalue".to_owned()));
        // Without a token the message goes without one.
        sqlx::query("UPDATE git_deploys SET sealed_token = NULL WHERE id = $1")
            .bind(deploy)
            .execute(&pool)
            .await
            .unwrap();
        let Operation::RunGitDeploy { token, .. } =
            operation_secrets(pool.clone(), key)(op).await.unwrap()
        else {
            panic!("the same kind of operation");
        };
        assert!(token.is_none());
    }
}
