//! Migration transfers over HTTP, against real PostgreSQL and Redis (T15.04, T15.06).
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::{io::Write as _, path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_migration::{MigrationState, admin_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Fake {
    manifest: Manifest,
    routes: Box<dyn Fn() -> ModuleRoutes + Send + Sync>,
}

impl Module for Fake {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn routes(&self) -> ModuleRoutes {
        (self.routes)()
    }
}

struct Env {
    app: Router,
    sessions: SessionStore,
    spool: PathBuf,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool) -> Env {
    env_with(pool, false).await
}

async fn env_with(pool: &PgPool, local: bool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let spool = std::env::temp_dir().join(format!("bp-migration-{}", Uuid::new_v4()));
    std::fs::create_dir_all(spool.join("migration")).unwrap();
    let manifest = Manifest::parse(
        "migration",
        "[module]\nname = \"backups\"\nversion = \"1.0.0\"\ntitle = { en = \"Migration\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions, spool) = (pool.clone(), sessions.clone(), spool.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(MigrationState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    spool_dir: spool.clone(),
                    loopback: Default::default(),
                    max_backup_bytes: 1 << 30,
                    allow_local_sources: local,
                })),
                ..ModuleRoutes::none()
            }
        }),
    });
    let registry = Registry::build(vec![module]).unwrap();
    let values: Vec<Value> = registry
        .modules()
        .iter()
        .map(|m| serde_json::to_value(m.manifest()).unwrap())
        .collect();
    let records: Vec<db::ModuleRecord<'_>> = registry
        .modules()
        .iter()
        .zip(&values)
        .map(|(m, json)| db::ModuleRecord {
            name: &m.manifest().module.name,
            version: &m.manifest().module.version,
            category: m.manifest().module.category.as_str(),
            core: m.manifest().module.core,
            manifest: json,
        })
        .collect();
    db::sync(pool, &records).await.unwrap();
    let mount = ModuleMount::new(registry, Router::new());
    mount.rebuild();
    mount.apply(&db::enabled_names(pool).await.unwrap().into_iter().collect());
    Env {
        app: mount.service(),
        sessions,
        spool,
    }
}

impl Env {
    async fn sign_in(&self, account_id: Uuid) -> Browser {
        let (t, data) = self
            .sessions
            .create(NewSession {
                account_id,
                ip: None,
                user_agent: None,
            })
            .await
            .unwrap();
        Browser {
            cookie: set_cookie(&t).split(';').next().unwrap().to_owned(),
            csrf: data.csrf_token.expose().to_owned(),
        }
    }

    async fn call(
        &self,
        who: &Browser,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
        let req = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                req.body(Body::from(v.to_string())).unwrap()
            }
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn account(pool: &PgPool, kind: &str, name: &str, roles: &[&str]) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ($3::account_kind, $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap();
    for role in roles {
        builtpanel_database::rbac::assign_role(pool, id, role, None)
            .await
            .unwrap();
    }
    id
}

fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, content) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(content.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut h, path, content.as_bytes()).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

fn cpmove() -> Vec<u8> {
    tar_gz(&[
        ("cpmove-alice/version", "pkgacct version 11.110\n"),
        (
            "cpmove-alice/cp/alice",
            "USER=alice\nDNS=example.com\nCONTACTEMAIL=alice@example.net\nPLAN=gold\nSTARTDATE=1700000000\nLOCALE=en\n",
        ),
        (
            "cpmove-alice/userdata/main",
            "---\nmain_domain: example.com\naddon_domains: {}\nparked_domains: []\nsub_domains: []\n",
        ),
        (
            "cpmove-alice/userdata/example.com",
            "---\ndocumentroot: /home/alice/public_html\nhomedir: /home/alice\nphpversion: ea-php81\n",
        ),
        (
            "cpmove-alice/mysql/alice_wp.sql",
            "-- dump\nCREATE TABLE t (id int);\n",
        ),
        (
            "cpmove-alice/homedir/public_html/index.php",
            "<?php echo 1;",
        ),
    ])
}

fn start(file: &str, username: &str, extra: Value) -> Value {
    let mut body = json!({
        "source_type": "cpanel", "backup_file": file, "tenant_username": username,
        "target_domain": "alice.example.org", "dry_run": true
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_dry_run_plans_the_import_and_changes_nothing(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "admin", "rootuser", &["super_admin"]).await)
        .await;
    let client = e
        .sign_in(account(&pool, "client", "carol", &[]).await)
        .await;
    let path = "/api/v1/admin/migration/transfers";
    std::fs::write(e.spool.join("migration/cpmove-alice.tar.gz"), cpmove()).unwrap();

    // Not for clients.
    let (status, _) = e.call(&client, "GET", path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let accounts_before: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (status, made) = e
        .call(
            &root,
            "POST",
            path,
            Some(start("cpmove-alice.tar.gz", "alice", json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["status"], "completed", "{made}");
    assert_eq!(made["progress_percent"], 100);
    assert_eq!(made["plan"]["ok"], true, "{made}");
    assert!(made["steps"].as_array().unwrap().len() >= 3, "{made}");
    assert!(made["size_bytes"].as_i64().unwrap() > 0);
    assert!(
        made["logs"].to_string().contains("nothing was changed"),
        "{made}"
    );
    // Nothing was created: a dry run only plans.
    let accounts_after: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(accounts_before, accounts_after);
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tenants, 0);

    // It is listed, can be read, and the audit trail has it.
    let id = made["id"].as_str().unwrap().to_owned();
    let (status, list) = e.call(&root, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (status, one) = e.call(&root, "GET", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["tenant_username"], "alice");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'migration.transfer_started'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);

    // A finished transfer cannot be cancelled; it can be deleted.
    let (status, got) = e
        .call(&root, "POST", &format!("{path}/{id}/cancel"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(code(&got), "not_cancellable");
    let (status, _) = e.call(&root, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = e.call(&root, "GET", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn names_that_are_taken_block_or_are_renamed(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "admin", "rootuser", &["super_admin"]).await)
        .await;
    account(&pool, "client", "alice", &[]).await;
    std::fs::write(e.spool.join("migration/cpmove-alice.tar.gz"), cpmove()).unwrap();
    let path = "/api/v1/admin/migration/transfers";

    let (status, blocked) = e
        .call(
            &root,
            "POST",
            path,
            Some(start("cpmove-alice.tar.gz", "alice", json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{blocked}");
    assert_eq!(blocked["plan"]["ok"], false, "{blocked}");
    assert!(blocked["logs"].to_string().contains("Blocked"), "{blocked}");

    let (status, renamed) = e
        .call(
            &root,
            "POST",
            path,
            Some(start(
                "cpmove-alice.tar.gz",
                "alice",
                json!({"on_conflict": "rename"}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{renamed}");
    assert_eq!(renamed["plan"]["ok"], true, "{renamed}");
    assert_eq!(renamed["plan"]["model"]["account"]["username"], "alice2");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn bad_requests_and_missing_files_are_refused_or_fail_cleanly(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "admin", "rootuser", &["super_admin"]).await)
        .await;
    let path = "/api/v1/admin/migration/transfers";
    for (body, field) in [
        (start("../etc/passwd", "alice", json!({})), "backup_file"),
        (start("a.tar.gz", "Al", json!({})), "tenant_username"),
        (
            start("a.tar.gz", "alice", json!({"source_type": "plesk"})),
            "source_type",
        ),
        (
            start("a.tar.gz", "alice", json!({"remote_host": "198.51.100.7"})),
            "auth_method",
        ),
        (
            start("a.tar.gz", "alice", json!({"on_conflict": "overwrite"})),
            "on_conflict",
        ),
    ] {
        let (status, got) = e.call(&root, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {got}");
        assert!(got.to_string().contains(field), "{field}: {got}");
    }
    // Without the panel's own routes to work through, a real import cannot start.
    let (status, got) = e
        .call(
            &root,
            "POST",
            path,
            Some(start("a.tar.gz", "alice", json!({"dry_run": false}))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(code(&got), "import_unavailable");

    // A file that is not there: the transfer exists and has failed, with the reason.
    let (status, failed) = e
        .call(
            &root,
            "POST",
            path,
            Some(start("missing.tar.gz", "alice", json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{failed}");
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["error_message"]
            .as_str()
            .unwrap()
            .contains("not in the import folder")
    );
    let id = failed["id"].as_str().unwrap().to_owned();

    // The file turns up: retry runs it again from the first step and it finishes.
    std::fs::write(e.spool.join("migration/missing.tar.gz"), cpmove()).unwrap();
    let (status, again) = e
        .call(&root, "POST", &format!("{path}/{id}/retry"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["status"], "completed", "{again}");
    assert!(again["error_message"].is_null());
    // Only a failed or cancelled transfer can be retried.
    let (status, got) = e
        .call(&root, "POST", &format!("{path}/{id}/retry"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");

    // A file of another panel kind is not read as the wrong one.
    std::fs::write(e.spool.join("migration/junk.tar.gz"), b"not an archive").unwrap();
    let (_, junk) = e
        .call(
            &root,
            "POST",
            path,
            Some(start("junk.tar.gz", "alice", json!({}))),
        )
        .await;
    assert_eq!(junk["status"], "failed", "{junk}");
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_failed_import_goes_on_from_its_step_and_a_dry_run_starts_over(pool: PgPool) {
    use builtpanel_database::migration_transfers as store;
    let admin = account(&pool, "admin", "rootuser", &["super_admin"]).await;
    let made = store::insert(
        &pool,
        &store::NewTransfer {
            source_type: "cpanel",
            source_host: None,
            backup_file: "a.tar.gz",
            tenant_username: "alice",
            target_domain: "alice.example.org",
            dry_run: false,
            on_conflict: "fail",
            created_by: admin,
        },
    )
    .await
    .unwrap();
    let steps = json!([
        {"id": "step-1", "name": "one", "status": "completed", "details": "kept"},
        {"id": "step-2", "name": "two", "status": "failed", "details": "boom"},
        {"id": "step-3", "name": "three", "status": "pending"}
    ]);
    // Without a plan there is nothing to go on from.
    store::set_progress(
        &pool,
        made.id,
        &store::Progress {
            status: "failed",
            progress_percent: 33,
            current_step: "two",
            size_bytes: None,
            steps: &steps,
            plan: None,
            error_message: Some("boom"),
        },
    )
    .await
    .unwrap();
    assert!(!store::resume(&pool, made.id).await.unwrap());
    let plan = json!({"ok": true});
    store::set_progress(
        &pool,
        made.id,
        &store::Progress {
            status: "failed",
            progress_percent: 33,
            current_step: "two",
            size_bytes: None,
            steps: &steps,
            plan: Some(&plan),
            error_message: Some("boom"),
        },
    )
    .await
    .unwrap();
    assert!(store::resume(&pool, made.id).await.unwrap());
    let t = store::get(&pool, made.id).await.unwrap().unwrap();
    assert_eq!(t.status, "running");
    assert!(t.error_message.is_none());
    let s = t.steps.as_array().unwrap();
    assert_eq!(s[0]["status"], "completed", "what was done stays done");
    assert_eq!(s[0]["details"], "kept");
    assert_eq!(
        s[1]["status"], "pending",
        "the step that failed is tried again"
    );
    assert!(s[1].get("details").is_none());
    // A transfer that is running cannot be resumed again.
    assert!(!store::resume(&pool, made.id).await.unwrap());
}

async fn wait_for_end(e: &Env, root: &Browser, id: &str) -> Value {
    for _ in 0..80 {
        let (_, t) = e
            .call(
                root,
                "GET",
                &format!("/api/v1/admin/migration/transfers/{id}"),
                None,
            )
            .await;
        if t["status"] != "running" && t["status"] != "pending" {
            return t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("the transfer did not end");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_backup_is_fetched_from_a_server_and_the_login_is_kept_nowhere(pool: PgPool) {
    let e = env_with(&pool, true).await;
    let root = e
        .sign_in(account(&pool, "admin", "rootuser", &["super_admin"]).await)
        .await;
    let (port, fingerprint) = common::serve(cpmove()).await;
    let path = "/api/v1/admin/migration/transfers";
    let body = |extra: Value| {
        let mut b = json!({
            "source_type": "cpanel", "tenant_username": "alice",
            "target_domain": "alice.example.org", "dry_run": true,
            "remote_host": "127.0.0.1", "remote_ssh_port": port, "remote_user": "alice",
            "auth_method": "password", "password": common::PASSWORD,
            "remote_backup_path": common::PATH
        });
        for (k, v) in extra.as_object().unwrap() {
            b[k] = v.clone();
        }
        b
    };

    // Without the host key nothing is sent: the transfer fails and names the key it saw.
    let (status, made) = e.call(&root, "POST", path, Some(body(json!({})))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["source_host"], "127.0.0.1");
    let first = made["id"].as_str().unwrap().to_owned();
    let failed = wait_for_end(&e, &root, &first).await;
    assert_eq!(failed["status"], "failed", "{failed}");
    let why = failed["error_message"].as_str().unwrap();
    assert!(
        why.contains(&fingerprint) && why.contains("host_key_sha256"),
        "{why}"
    );
    // It cannot be run again without the login, which was kept nowhere.
    let (status, got) = e
        .call(&root, "POST", &format!("{path}/{first}/retry"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(code(&got), "login_needed");

    // With it, the backup comes over and is planned.
    let (status, made) = e
        .call(
            &root,
            "POST",
            path,
            Some(body(json!({"host_key_sha256": fingerprint}))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_owned();
    let done = wait_for_end(&e, &root, &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["plan"]["ok"], true, "{done}");
    assert!(done["logs"].to_string().contains("Fetched"), "{done}");
    let saved = done["backup_file"].as_str().unwrap();
    assert!(saved.starts_with("remote-") && e.spool.join("migration").join(saved).exists());

    // The password is in no row: not in the transfers, not in the audit trail.
    for table in ["migration_transfers", "audit_logs"] {
        let sql = format!("SELECT count(*) FROM {table} t WHERE row_to_json(t)::text LIKE $1");
        let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(format!("%{}%", common::PASSWORD))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(hits, 0, "{table} holds the password");
    }

    // A wrong password fails with a message that does not repeat it.
    let (_, wrong) = e
        .call(
            &root,
            "POST",
            path,
            Some(body(
                json!({"host_key_sha256": fingerprint, "password": "hunter2-wrong"}),
            )),
        )
        .await;
    let ended = wait_for_end(&e, &root, wrong["id"].as_str().unwrap()).await;
    assert_eq!(ended["status"], "failed");
    assert!(!ended.to_string().contains("hunter2-wrong"));

    // A server on this machine is refused when the panel is not in test mode.
    let strict = env_with(&pool, false).await;
    let root2 = strict
        .sign_in(account(&pool, "admin", "second", &["super_admin"]).await)
        .await;
    let (_, refused) = strict
        .call(
            &root2,
            "POST",
            path,
            Some(body(json!({"host_key_sha256": fingerprint}))),
        )
        .await;
    let ended = wait_for_end(&strict, &root2, refused["id"].as_str().unwrap()).await;
    assert_eq!(ended["status"], "failed");
    assert!(
        ended["error_message"]
            .as_str()
            .unwrap()
            .contains("cannot be used"),
        "{ended}"
    );
}
