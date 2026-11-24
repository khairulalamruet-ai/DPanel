//! Backup destinations over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_backups::{BackupsState, admin_router, client_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::SettingsRegistry;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const KEY: [u8; 32] = [9u8; 32];

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
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let settings = Arc::new(SettingsRegistry::build(vec![]).unwrap());
    let manifest = Manifest::parse(
        "backups",
        "[module]\nname = \"backups\"\nversion = \"1.0.0\"\ntitle = { en = \"Backups\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(BackupsState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    settings: settings.clone(),
                    master_key: MasterKey::from_bytes(&KEY),
                })),
                client: Some(client_router().with_state(BackupsState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    settings: settings.clone(),
                    master_key: MasterKey::from_bytes(&KEY),
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

async fn admin(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn destinations_keep_their_secrets_sealed_and_never_show_them(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let nobody = e.sign_in(admin(&pool, "plain", &[]).await).await;
    let path = "/api/v1/admin/backup-destinations";
    let (status, _) = e.call(&nobody, "GET", path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A folder on the node: nothing secret; the first one is the default.
    let (status, local) = e.call(&root, "POST", path, Some(json!({"name": "Disk", "kind": "local", "settings": {"path": "/var/backups/builtpanel"}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{local}");
    assert_eq!(
        (
            local["is_default"].clone(),
            local["has_credentials"].clone()
        ),
        (json!(true), json!(false))
    );
    // S3: the keys are sealed at once.
    let secret = "super-secret-key-1234567890";
    let (status, s3) = e
        .call(&root, "POST", path, Some(json!({
            "name": "Offsite",
            "kind": "s3",
            "settings": {"endpoint": "https://s3.example.com", "bucket": "bp-backups", "prefix": "/panel/", "ignored": "x"},
            "credentials": {"access_key": "AKIAEXAMPLE", "secret_key": secret}
        })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{s3}");
    assert_eq!(
        (s3["has_credentials"].clone(), s3["is_default"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(
        s3["settings"],
        json!({"endpoint": "https://s3.example.com", "bucket": "bp-backups", "region": "us-east-1", "prefix": "panel", "path_style": true})
    );
    let id: Uuid = s3["id"].as_str().unwrap().parse().unwrap();
    // Nowhere in the database or the answers is the secret in plain.
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_credentials FROM backup_destinations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.windows(secret.len()).any(|w| w == secret.as_bytes()));
    let everything: String =
        sqlx::query_scalar("SELECT d::text FROM backup_destinations d WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!everything.contains(secret) && !everything.contains("AKIAEXAMPLE"));
    let opened = MasterKey::from_bytes(&KEY)
        .open(
            &builtpanel_database::backups::credentials_context(id),
            &stored,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&opened).unwrap()["secret_key"],
        secret
    );
    let (_, list) = e.call(&root, "GET", path, None).await;
    assert!(!list.to_string().contains(secret) && !list.to_string().contains("AKIA"));
    // Sealed for that destination only.
    assert!(
        MasterKey::from_bytes(&KEY)
            .open(
                &builtpanel_database::backups::credentials_context(Uuid::new_v4()),
                &stored
            )
            .is_err()
    );

    // What cannot be.
    for (body, why) in [
        (
            json!({"name": "x", "kind": "ftp", "settings": {}}),
            "invalid_kind",
        ),
        (
            json!({"name": "x", "kind": "local", "settings": {"path": "relative"}}),
            "invalid_path",
        ),
        (
            json!({"name": "x", "kind": "local", "settings": {"path": "/a/../b"}}),
            "invalid_path",
        ),
        (
            json!({"name": "x", "kind": "s3", "settings": {"endpoint": "http://evil.example.com", "bucket": "abc"}, "credentials": {"access_key": "a", "secret_key": "b"}}),
            "invalid_endpoint",
        ),
        (
            json!({"name": "x", "kind": "s3", "settings": {"endpoint": "https://s3.example.com", "bucket": "AB"}, "credentials": {"access_key": "a", "secret_key": "b"}}),
            "invalid_bucket",
        ),
        (
            json!({"name": "x", "kind": "s3", "settings": {"endpoint": "https://s3.example.com", "bucket": "abc"}}),
            "required",
        ),
        (
            json!({"name": "x", "kind": "sftp", "settings": {"host": "h", "user": "u"}, "credentials": {"password": "p", "private_key": "-----BEGIN PRIVATE KEY-----"}}),
            "invalid_credentials",
        ),
        (
            json!({"name": "x", "kind": "sftp", "settings": {"host": "h", "user": "u", "host_key_sha256": "zz"}, "credentials": {"password": "p"}}),
            "invalid_fingerprint",
        ),
        (
            json!({"name": "", "kind": "local", "settings": {"path": "/x"}}),
            "invalid_name",
        ),
    ] {
        let (status, got) = e.call(&root, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {got}");
        assert!(got.to_string().contains(why), "{why}: {got}");
    }
    let (status, got) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"name": "Disk", "kind": "local", "settings": {"path": "/x"}})),
        )
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "name_taken"));

    // Changing: new keys replace the sealed ones; the default is moved, not switched off.
    let (status, got) = e.call(&root, "PUT", &format!("{path}/{id}"), Some(json!({"credentials": {"access_key": "NEWKEY", "secret_key": "new-secret-9876543210"}, "enabled": true}))).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let stored2: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_credentials FROM backup_destinations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(stored, stored2);
    let local_id = local["id"].as_str().unwrap();
    let (status, got) = e
        .call(
            &root,
            "PUT",
            &format!("{path}/{local_id}"),
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "is_default"));
    let (status, got) = e
        .call(&root, "DELETE", &format!("{path}/{local_id}"), None)
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "is_default"));
    let (status, _) = e
        .call(&root, "POST", &format!("{path}/{id}/default"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    // A destination backups point at stays.
    let tenant_free: Uuid = sqlx::query_scalar(
        "INSERT INTO backups (kind, trigger, destination_id, status) VALUES ('system', 'manual', $1, 'ready') RETURNING id",
    )
    .bind(local_id.parse::<Uuid>().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, got) = e
        .call(&root, "DELETE", &format!("{path}/{local_id}"), None)
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "in_use"));
    sqlx::query("DELETE FROM backups WHERE id = $1")
        .bind(tenant_free)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = e
        .call(&root, "DELETE", &format!("{path}/{local_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'backup_destination.%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        actions.iter().any(|a| a == "backup_destination.added")
            && actions.iter().any(|a| a == "backup_destination.removed")
    );
    assert!(!actions.is_empty());
}

async fn client(pool: &PgPool, name: &str, uid: i32) -> (Uuid, Uuid) {
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap();
    (account, tenant)
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_owner_makes_lists_restores_and_removes_backups_and_sets_a_schedule(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, tenant) = client(&pool, "alice", 10_000).await;
    let (bob_account, _) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let bob = e.sign_in(bob_account).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;

    // No place for backups yet.
    let (status, body) = e.call(&alice, "POST", "/api/v1/client/backups", None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "no_destination"),
        "{body}"
    );
    e.call(
        &root,
        "POST",
        "/api/v1/admin/backup-destinations",
        Some(
            json!({"name": "Disk", "kind": "local", "settings": {"path": "/var/backups/builtpanel"}}),
        ),
    )
    .await;

    let (status, made) = e.call(&alice, "POST", "/api/v1/client/backups", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(
        (made["status"].as_str(), made["trigger"].as_str()),
        (Some("queued"), Some("manual"))
    );
    // One at a time.
    let (status, body) = e.call(&alice, "POST", "/api/v1/client/backups", None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "backup_active")
    );
    // Nobody else sees it, touches it or restores it.
    let (_, list) = e.call(&bob, "GET", "/api/v1/client/backups", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0);
    let (status, _) = e
        .call(
            &bob,
            "DELETE",
            &format!("/api/v1/client/backups/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The backup has a key of its own, sealed.
    let sealed: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT sealed_key FROM backups WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sealed.is_some_and(|k| k.len() > 32));
    // Not ready: not removable, not restorable.
    let (status, body) = e
        .call(
            &alice,
            "DELETE",
            &format!("/api/v1/client/backups/{id}"),
            None,
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "backup_busy"));
    let restore = format!("/api/v1/client/backups/{id}/restore");
    let (status, body) = e
        .call(&alice, "POST", &restore, Some(json!({"files": true})))
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "backup_not_ready")
    );

    // The node finishes it.
    sqlx::query("UPDATE backups SET status = 'ready', size_bytes = 99, finished_at = now(), parts = $2 WHERE id = $1::uuid")
        .bind(&id)
        .bind(json!([
            {"name": "files.tar.zst.enc", "kind": "files", "stored_bytes": 50, "sha256": "a".repeat(64), "database": null},
            {"name": "db-alice_shop.sql.zst.enc", "kind": "database", "stored_bytes": 49, "sha256": "b".repeat(64), "database": "alice_shop"}
        ]))
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = e.call(&alice, "GET", "/api/v1/client/backups", None).await;
    assert_eq!(
        list["items"][0]["parts"].as_array().unwrap().len(),
        2,
        "{list}"
    );
    // What cannot be restored is refused.
    for (body, want) in [
        (json!({}), "nothing_to_restore"),
        (json!({"databases": ["other_db"]}), "not_in_backup"),
        (json!({"files": true, "folder": "../etc"}), "invalid"),
        (json!({"files": false, "folder": "www"}), "invalid"),
    ] {
        let (status, got) = e.call(&alice, "POST", &restore, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {got}");
        assert!(got.to_string().contains(want), "{body}: {got}");
    }
    let (status, started) = e
        .call(
            &alice,
            "POST",
            &restore,
            Some(json!({"files": true, "folder": "/www/blog/", "databases": ["alice_shop"]})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(
        (started["status"].as_str(), started["folder"].as_str()),
        (Some("queued"), Some("www/blog"))
    );
    // One at a time.
    let (status, body) = e
        .call(&alice, "POST", &restore, Some(json!({"files": true})))
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "restore_active")
    );
    let (_, restores) = e
        .call(&alice, "GET", "/api/v1/client/backup-restores", None)
        .await;
    assert_eq!(restores["items"].as_array().unwrap().len(), 1);

    // Removing it starts the removal.
    let (status, _) = e
        .call(
            &alice,
            "DELETE",
            &format!("/api/v1/client/backups/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let s: String = sqlx::query_scalar("SELECT status FROM backups WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(s, "deleting");

    // The schedule.
    let sched = "/api/v1/client/backup-schedule";
    let (status, _) = e.call(&alice, "GET", sched, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for (body, want) in [
        (
            json!({"cron": "* * * * *", "keep_daily": 7, "keep_weekly": 4, "enabled": true}),
            "too_often",
        ),
        (
            json!({"cron": "nonsense", "keep_daily": 7, "keep_weekly": 4, "enabled": true}),
            "invalid",
        ),
        (
            json!({"cron": "30 2 * * *", "keep_daily": 99, "keep_weekly": 4, "enabled": true}),
            "out_of_range",
        ),
    ] {
        let (status, got) = e.call(&alice, "PUT", sched, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {got}");
        assert!(got.to_string().contains(want), "{body}: {got}");
    }
    let (status, saved) = e
        .call(
            &alice,
            "PUT",
            sched,
            Some(json!({"cron": "30 2 * * *", "keep_daily": 7, "keep_weekly": 4, "enabled": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(saved["next_run_at"].is_string());
    let due: bool =
        sqlx::query_scalar("SELECT next_run_at > now() FROM backup_schedules WHERE tenant_id = $1")
            .bind(tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(due);
    let (status, _) = e.call(&alice, "DELETE", sched, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Not a client: no.
    let (status, _) = e.call(&root, "GET", "/api/v1/client/backups", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A plan without the feature has none.
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, body) = e.call(&alice, "GET", "/api/v1/client/backups", None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn administrators_make_list_and_remove_system_backups_and_set_their_schedule(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let nobody = e.sign_in(admin(&pool, "plain", &[]).await).await;
    let path = "/api/v1/admin/system-backups";
    let (status, _) = e.call(&nobody, "GET", path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = e.call(&root, "POST", path, None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "no_destination")
    );
    e.call(
        &root,
        "POST",
        "/api/v1/admin/backup-destinations",
        Some(
            json!({"name": "Disk", "kind": "local", "settings": {"path": "/var/backups/builtpanel"}}),
        ),
    )
    .await;
    let (status, made) = e.call(&root, "POST", path, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (made["status"].as_str(), made["trigger"].as_str()),
        (Some("queued"), Some("manual"))
    );
    let (status, body) = e.call(&root, "POST", path, None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "backup_active")
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let (status, body) = e.call(&root, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "backup_busy"));
    sqlx::query("UPDATE backups SET status = 'ready' WHERE id = $1::uuid")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = e.call(&root, "GET", path, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let (status, _) = e.call(&root, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The schedule.
    let sched = "/api/v1/admin/system-backup-schedule";
    let (status, _) = e.call(&root, "GET", sched, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = e
        .call(
            &root,
            "PUT",
            sched,
            Some(
                json!({"cron": "*/5 * * * *", "keep_daily": 7, "keep_weekly": 4, "enabled": true}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (status, saved) = e
        .call(
            &root,
            "PUT",
            sched,
            Some(
                json!({"cron": "0 3 * * *", "keep_daily": 30, "keep_weekly": 12, "enabled": true}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, _) = e
        .call(
            &nobody,
            "PUT",
            sched,
            Some(
                json!({"cron": "0 3 * * *", "keep_daily": 30, "keep_weekly": 12, "enabled": true}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = e.call(&root, "DELETE", sched, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_account_export_is_asked_for_and_turned_into_a_backup_of_the_same_account(pool: PgPool) {
    use builtpanel_backup::export::Header;
    let e = env(&pool).await;
    let (alice_account, alice_tenant) = client(&pool, "alice", 10_000).await;
    let (_, bob_tenant) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let dir = std::env::temp_dir().join(format!("bp-export-api-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("exports")).unwrap();
    let (_, dest) = e
        .call(&root, "POST", "/api/v1/admin/backup-destinations", Some(json!({"name": "Disk", "kind": "local", "settings": {"path": dir.to_str().unwrap()}})))
        .await;
    let dest_id: Uuid = dest["id"].as_str().unwrap().parse().unwrap();
    // A finished backup of alice, with its key sealed as the runner does.
    let key = [4u8; 32];
    let backup: Uuid = sqlx::query_scalar(
        "INSERT INTO backups (tenant_id, account_name, kind, trigger, destination_id, status, size_bytes)
         VALUES ($1, 'alice', 'tenant', 'manual', $2, 'ready', 10) RETURNING id",
    )
    .bind(alice_tenant)
    .bind(dest_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let sealed = MasterKey::from_bytes(&KEY)
        .seal(&builtpanel_database::backups::key_context(backup), &key)
        .unwrap();
    builtpanel_database::backups::keep_key(&pool, backup, &sealed)
        .await
        .unwrap();

    let export = format!("/api/v1/client/backups/{backup}/export");
    let (status, body) = e
        .call(
            &alice,
            "POST",
            &export,
            Some(json!({"passphrase": "short"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
        "{body}"
    );
    let (status, made) = e
        .call(
            &alice,
            "POST",
            &export,
            Some(json!({"passphrase": "a long enough passphrase"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(made["status"], "queued");
    let (status, body) = e
        .call(
            &alice,
            "POST",
            &export,
            Some(json!({"passphrase": "a long enough passphrase"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "export_active")
    );
    // The passphrase is nowhere, and the key is only in the wrapped header.
    let header: String = sqlx::query_scalar("SELECT header FROM backup_exports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!header.contains("a long enough"));
    let read = Header::from_reader(header.as_bytes()).unwrap();
    assert_eq!(read.unwrap_key("a long enough passphrase").unwrap(), key);
    let (_, list) = e
        .call(&alice, "GET", "/api/v1/client/backup-exports", None)
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    // The runner gives it to the node, without the credentials in the ledger.
    let master = MasterKey::from_bytes(&KEY);
    builtpanel_mod_backups::runner::tick(&pool, &master, 1_710_000_000)
        .await
        .unwrap();
    let op: (String, String) =
        sqlx::query_as("SELECT kind, idempotency_key FROM operations WHERE kind = 'export_backup'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(op.1.starts_with("export:"));
    let id: Uuid = sqlx::query_scalar("SELECT id FROM operations WHERE kind = 'export_backup'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1, dispatched_at = now() WHERE id = $1").bind(id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
        .bind(id)
        .bind(json!({"type": "backup_exported", "object": "exports/abc.tar", "size_bytes": 77}))
        .execute(&pool)
        .await
        .unwrap();
    let (status, object, size): (String, Option<String>, Option<i64>) =
        sqlx::query_as("SELECT status, object, size_bytes FROM backup_exports")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), object.as_deref(), size),
        ("ready", Some("exports/abc.tar"), Some(77))
    );

    // The other side: a file at the destination, made by another panel, imported.
    let other_backup = Uuid::new_v4();
    let h = Header::new(
        &other_backup.simple().to_string(),
        "alice",
        1_700_000_000,
        &[8u8; 32],
        "another long passphrase",
    )
    .unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    for (name, body) in [
        ("export.json", h.to_json().unwrap()),
        ("manifest.json.enc", b"sealed".to_vec()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o600);
        tar.append_data(&mut header, name, &body[..]).unwrap();
    }
    std::fs::write(dir.join("exports/fromother.tar"), tar.into_inner().unwrap()).unwrap();
    let imports = "/api/v1/admin/backup-imports";
    let ok = |tenant: Uuid, object: &str, pass: &str| json!({"tenant_id": tenant, "destination_id": dest_id, "object": object, "passphrase": pass});
    for (body, want) in [
        (
            ok(alice_tenant, "../etc/passwd", "another long passphrase"),
            "invalid",
        ),
        (
            ok(
                alice_tenant,
                "exports/fromother.tar",
                "the wrong passphrase!",
            ),
            "wrong_passphrase",
        ),
        (
            ok(
                bob_tenant,
                "exports/fromother.tar",
                "another long passphrase",
            ),
            "account_mismatch",
        ),
    ] {
        let (status, got) = e.call(&root, "POST", imports, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {got}");
        assert!(got.to_string().contains(want), "{body}: {got}");
    }
    let (status, _) = e
        .call(
            &root,
            "POST",
            imports,
            Some(ok(alice_tenant, "exports/missing.tar", "x")),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    std::fs::write(dir.join("exports/notatar.tar"), b"plain text, not a tar").unwrap();
    let (status, got) = e
        .call(
            &root,
            "POST",
            imports,
            Some(ok(alice_tenant, "exports/notatar.tar", "x")),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    let (status, started) = e
        .call(
            &root,
            "POST",
            imports,
            Some(ok(
                alice_tenant,
                "exports/fromother.tar",
                "another long passphrase",
            )),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["status"], "import_queued");
    assert_eq!(started["id"].as_str().unwrap(), other_backup.to_string());
    let (status, body) = e
        .call(
            &root,
            "POST",
            imports,
            Some(ok(
                alice_tenant,
                "exports/fromother.tar",
                "another long passphrase",
            )),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "backup_exists")
    );
    // Its key is the other panel's, sealed with this one's master key.
    let opened = builtpanel_mod_backups::runner::fill(
        &pool,
        &master,
        builtpanel_node_protocol::Operation::ImportExport {
            object: "exports/fromother.tar".into(),
            backup_id: other_backup,
            target: builtpanel_node_protocol::BackupTarget {
                kind: "local".into(),
                settings: json!({}),
            },
            credentials: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        opened,
        builtpanel_node_protocol::Operation::ImportExport { .. }
    ));
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT sealed_key FROM backups WHERE id = $1")
        .bind(other_backup)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        master
            .open(
                &builtpanel_database::backups::key_context(other_backup),
                &sealed
            )
            .unwrap(),
        [8u8; 32]
    );
    // The runner queues the unpacking; the answer makes the backup ready.
    builtpanel_mod_backups::runner::tick(&pool, &master, 1_710_000_000)
        .await
        .unwrap();
    let id: Uuid = sqlx::query_scalar("SELECT id FROM operations WHERE kind = 'import_export'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1, dispatched_at = now() WHERE id = $1").bind(id).execute(&pool).await.unwrap();
    let s: String = sqlx::query_scalar("SELECT status FROM backups WHERE id = $1")
        .bind(other_backup)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(s, "importing");
    sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
        .bind(id)
        .bind(json!({"type": "export_imported", "size_bytes": 55,
                     "parts": [{"name": "files.tar.zst.enc", "kind": "files", "stored_bytes": 55, "sha256": "c".repeat(64), "database": null}]}))
        .execute(&pool)
        .await
        .unwrap();
    let (status, parts): (String, Value) =
        sqlx::query_as("SELECT status, parts FROM backups WHERE id = $1")
            .bind(other_backup)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), parts.as_array().unwrap().len()),
        ("ready", 1)
    );
    let (_, mine) = e.call(&alice, "GET", "/api/v1/client/backups", None).await;
    assert!(
        mine["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["id"] == other_backup.to_string() && b["status"] == "ready")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the node does for a transfer's step, done by hand: an operation succeeds.
async fn node_did(pool: &PgPool, kind: &str, node: Uuid) -> Value {
    let (id, payload): (Uuid, Value) = sqlx::query_as(
        "SELECT id, payload FROM operations WHERE kind = $1 AND node_id = $2 AND status = 'pending'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(kind)
    .bind(node)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("no pending {kind} for that node: {e}"));
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        builtpanel_database::operations::succeed(pool, id, &json!({}))
            .await
            .unwrap()
    );
    payload
}

/// A step the node could not do.
async fn node_failed(pool: &PgPool, kind: &str, node: Uuid, message: &str) {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operations WHERE kind = $1 AND node_id = $2 AND status = 'pending'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(kind)
    .bind(node)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    builtpanel_database::operations::fail(
        pool,
        id,
        &json!({"code": "internal", "message": message, "retryable": false}),
        None,
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_account_is_copied_to_another_node_while_it_runs_then_frozen_for_the_last_pass_and_switched(
    pool: PgPool,
) {
    use builtpanel_mod_backups::{runner, transfer};
    let e = env(&pool).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let (_, tenant) = client(&pool, "dina", 10_000).await;
    let old: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET public_ip = '203.0.113.1', status = 'online' WHERE id = $1")
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();
    let new: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, public_ip, status) VALUES ('node2', '203.0.113.2', 'online') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let dest: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // What the account has: a site with a DNS record that names its node, a database and a
    // user with access (and a remote address), a cron job.
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'dina.example', 'primary') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let zone: Uuid = sqlx::query_scalar(
        "INSERT INTO dns_zones (domain_id, name) VALUES ($1, 'dina.example') RETURNING id",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap();
    for (name, rtype, content) in [("@", "A", "203.0.113.1"), ("@", "TXT", "203.0.113.1")] {
        sqlx::query("INSERT INTO dns_records (zone_id, name, rtype, content, ttl) VALUES ($1, $2, $3, $4, 300)")
            .bind(zone)
            .bind(name)
            .bind(rtype)
            .bind(content)
            .execute(&pool)
            .await
            .unwrap();
    }
    let database: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'dina_shop', 'ready') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO db_users (tenant_id, name, status) VALUES ($1, 'dina_web', 'ready') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO db_grants (tenant_id, database_id, user_id, access) VALUES ($1, $2, $3, 'all')")
        .bind(tenant)
        .bind(database)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO db_remote_hosts (user_id, cidr) VALUES ($1, '198.51.100.9/32')")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cron_jobs (tenant_id, name, schedule, command) VALUES ($1, 'tick', '*/10 * * * *', 'php cron.php')")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    let key = MasterKey::from_bytes(&KEY);
    let path = "/api/v1/admin/node-transfers";

    // Not to the same node, not to one that is not online, not an account that is not there.
    let (status, body) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": old})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "same_node"),
        "{body}"
    );
    sqlx::query("UPDATE nodes SET status = 'offline' WHERE id = $1")
        .bind(new)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": new})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "node_unavailable")
    );
    sqlx::query("UPDATE nodes SET status = 'online' WHERE id = $1")
        .bind(new)
        .execute(&pool)
        .await
        .unwrap();
    // What a transfer cannot carry yet is named and refused.
    sqlx::query("INSERT INTO ftp_accounts (tenant_id, name, home) VALUES ($1, 'dina_site', 'www')")
        .bind(tenant)
        .execute(&pool)
        .await
        .ok();
    let ftp: i64 = sqlx::query_scalar("SELECT count(*) FROM ftp_accounts WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    if ftp > 0 {
        let (status, body) = e
            .call(
                &root,
                "POST",
                path,
                Some(json!({"tenant_id": tenant, "to_node_id": new})),
            )
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::CONFLICT, "transfer_unsupported"),
            "{body}"
        );
        sqlx::query("DELETE FROM ftp_accounts WHERE tenant_id = $1")
            .bind(tenant)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Start: the account is set up on the new node first.
    let (status, started) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": new})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    let id = started["id"].as_str().unwrap().to_owned();
    let (status, body) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": new})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "transfer_active")
    );
    let state = || {
        let pool = pool.clone();
        let id = id.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT state FROM tenant_transfers WHERE id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "provisioning");
    let setup = node_did(&pool, "set_up_tenant", new).await;
    assert_eq!(setup["username"], "dina");

    // The first copy: a backup on the OLD node that carries the users' passwords, restored on
    // the NEW node, which makes the databases and the users.
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "precopy_backup");
    runner::tick(&pool, &key, 1_710_000_000).await.unwrap();
    let first = node_did(&pool, "create_backup", old).await;
    assert!(first.get("files_since").is_none(), "{first}");
    assert_eq!(first["db_users"][0]["name"], "dina_web");
    assert_eq!(first["databases"], json!(["dina_shop"]));
    sqlx::query("UPDATE backups SET status = 'ready' WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "precopy_restore");
    runner::tick(&pool, &key, 1_710_000_001).await.unwrap();
    let restore = node_did(&pool, "restore_backup", new).await;
    assert_eq!(restore["create_databases"], true);
    assert_eq!(restore["restore_db_users"], true);
    assert_eq!(restore["files"], true);
    sqlx::query("UPDATE backup_restores SET status = 'done' WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    // The site still runs on the old node and the account still belongs to it.
    assert_eq!(state().await, "precopied");
    let owner: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, old);
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "precopied", "it waits to be told to go on");

    // Cutover: frozen on the old node; the last backup carries only what changed.
    let (status, body) = e
        .call(&root, "POST", &format!("{path}/{id}/cleanup"), None)
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "wrong_state"));
    let (status, _) = e
        .call(&root, "POST", &format!("{path}/{id}/cutover"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "freezing");
    let frozen = node_did(&pool, "suspend_tenant", old).await;
    assert_eq!(frozen["username"], "dina");
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "delta_backup");
    // The first copy is old now (the backup row must not be a live one for the next).
    runner::tick(&pool, &key, 1_710_000_100).await.unwrap();
    let delta = node_did(&pool, "create_backup", old).await;
    assert!(delta["files_since"].as_i64().is_some(), "{delta}");
    sqlx::query("UPDATE backups SET status = 'ready' WHERE status IN ('queued', 'running')")
        .execute(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "delta_restore");
    runner::tick(&pool, &key, 1_710_000_101).await.unwrap();
    node_did(&pool, "restore_backup", new).await;
    sqlx::query("UPDATE backup_restores SET status = 'done' WHERE status IN ('queued', 'running')")
        .execute(&pool)
        .await
        .unwrap();

    // The switch: the account is the new node's; the sites, PHP, jobs, grants and the DNS
    // record that named the old address follow.
    let routes_before: i64 = sqlx::query_scalar("SELECT routes_version FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "switched");
    let owner: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, new);
    let routes_after: i64 = sqlx::query_scalar("SELECT routes_version FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(routes_after > routes_before);
    let records: Vec<(String, String)> =
        sqlx::query_as("SELECT rtype, content FROM dns_records WHERE zone_id = $1 ORDER BY rtype")
            .bind(zone)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        records,
        [
            ("A".to_owned(), "203.0.113.2".to_owned()),
            ("TXT".to_owned(), "203.0.113.1".to_owned())
        ],
        "only an address record that named the old node's address is changed"
    );
    let cron = node_did(&pool, "sync_cron_jobs", new).await;
    assert_eq!(cron["jobs"][0]["command"], "php cron.php");
    let grant = node_did(&pool, "grant_db", new).await;
    assert_eq!(
        (grant["database"].as_str(), grant["access"].as_str()),
        (Some("dina_shop"), Some("all"))
    );
    let remote = node_did(&pool, "set_db_remote_hosts", new).await;
    assert_eq!(remote["cidrs"], json!(["198.51.100.9/32"]));
    let on_new: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'set_db_remote_access' AND node_id = $1",
    )
    .bind(new)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(on_new["cidrs"], json!(["198.51.100.9/32"]));
    let (_, shown) = e.call(&root, "GET", &format!("{path}/{id}"), None).await;
    assert_eq!(shown["state"], "switched");
    assert!(
        shown["downtime_secs"].as_i64().is_some_and(|s| s >= 0),
        "{shown}"
    );

    // The old copy stays until someone says; then it goes, and the transfer's own backups with it.
    let (status, _) = e
        .call(&root, "POST", &format!("{path}/{id}/cleanup"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let gone = node_did(&pool, "delete_tenant", old).await;
    assert_eq!(gone["purge_data"], true);
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "done");
    let leftover: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM backups WHERE tenant_id = $1 AND status NOT IN ('deleting', 'deleted')",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leftover, 0);
    let audit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action LIKE 'node_transfer.%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audit, 3, "started, cut over, cleaned up");
    let _ = dest;
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_failure_after_the_freeze_gives_the_old_node_its_account_back_and_a_cancel_removes_the_copy(
    pool: PgPool,
) {
    use builtpanel_mod_backups::{runner, transfer};
    let e = env(&pool).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let (_, tenant) = client(&pool, "erin", 10_001).await;
    let old: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    let new: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, public_ip, status) VALUES ('node2', '203.0.113.2', 'online') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let key = MasterKey::from_bytes(&KEY);
    let path = "/api/v1/admin/node-transfers";
    let (_, started) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": new, "auto_cutover": true})),
        )
        .await;
    let id = started["id"].as_str().unwrap().to_owned();
    let state = || {
        let pool = pool.clone();
        let id = id.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT state FROM tenant_transfers WHERE id = $1::uuid",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    // All the way to the freeze by itself.
    transfer::tick(&pool, &key).await.unwrap();
    node_did(&pool, "set_up_tenant", new).await;
    transfer::tick(&pool, &key).await.unwrap();
    runner::tick(&pool, &key, 1_710_000_000).await.unwrap();
    node_did(&pool, "create_backup", old).await;
    sqlx::query("UPDATE backups SET status = 'ready'")
        .execute(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    runner::tick(&pool, &key, 1_710_000_001).await.unwrap();
    node_did(&pool, "restore_backup", new).await;
    sqlx::query("UPDATE backup_restores SET status = 'done'")
        .execute(&pool)
        .await
        .unwrap();
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "precopied");
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "freezing");
    node_did(&pool, "suspend_tenant", old).await;
    transfer::tick(&pool, &key).await.unwrap();
    runner::tick(&pool, &key, 1_710_000_100).await.unwrap();
    // The last backup fails: the old node gets its account back and nothing is switched.
    node_failed(&pool, "create_backup", old, "the destination is full").await;
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "failed");
    let resumed = node_did(&pool, "resume_tenant", old).await;
    assert_eq!(resumed["username"], "erin");
    let owner: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, old);
    let (_, shown) = e.call(&root, "GET", &format!("{path}/{id}"), None).await;
    assert!(
        shown["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{shown}"
    );

    // A failed transfer's half-made copy is removed with a cancel, and a new one can start.
    let (status, _) = e
        .call(&root, "POST", &format!("{path}/{id}/cancel"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let removed = node_did(&pool, "delete_tenant", new).await;
    assert_eq!(removed["purge_data"], true);
    transfer::tick(&pool, &key).await.unwrap();
    assert_eq!(state().await, "cancelled");
    let (status, _) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"tenant_id": tenant, "to_node_id": new})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}
