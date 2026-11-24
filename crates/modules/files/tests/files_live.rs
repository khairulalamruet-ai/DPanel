//! The file manager over HTTP against real PostgreSQL and Redis, with the node
//! played by the real file operations on a temporary directory.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_files::{FilesState, SETTINGS_SCHEMA, client_router, public_router};
use builtpanel_node_protocol::{Operation, Output};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::{Scope, SettingsRegistry};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const KEY: [u8; 32] = [9u8; 32];

fn spool_dir() -> PathBuf {
    std::env::temp_dir().join(format!("bp-files-spool-{}", std::process::id()))
}

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
    settings: Arc<SettingsRegistry>,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool, wait: Duration) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let settings = Arc::new(
        SettingsRegistry::build(vec![(
            "files".to_owned(),
            serde_json::from_str(SETTINGS_SCHEMA).unwrap(),
        )])
        .unwrap(),
    );
    let manifest = Manifest::parse(
        "files",
        "[module]\nname = \"files\"\nversion = \"1.0.0\"\ntitle = { en = \"Files\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions, settings) = (pool.clone(), sessions.clone(), settings.clone());
            move || {
                let state = FilesState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    settings: settings.clone(),
                    wait,
                    master_key: MasterKey::from_bytes(&KEY),
                    spool_dir: spool_dir(),
                    agent_url: Some("https://panel.example.com:8443".to_owned()),
                };
                ModuleRoutes {
                    client: Some(client_router().with_state(state.clone())),
                    public: Some(public_router().with_state(state)),
                    ..ModuleRoutes::none()
                }
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
        settings,
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
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn raw(
        &self,
        who: Option<&Browser>,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(who) = who {
            req = req
                .header(header::COOKIE, &who.cookie)
                .header(CSRF_HEADER, &who.csrf);
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 24)
            .await
            .unwrap();
        (status, headers, bytes.to_vec())
    }

    async fn set(&self, pool: &PgPool, values: Value) {
        let mut tx = pool.begin().await.unwrap();
        builtpanel_settings::set(
            &self.settings,
            &mut tx,
            "files",
            Scope::Global,
            values.as_object().unwrap(),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
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

/// Plays a node: each tenant has a directory under `base`, and every `file_op` is done by
/// the real operations on it.
fn node(pool: PgPool, base: PathBuf, delay: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let rows: Vec<(Uuid, Value)> = sqlx::query_as(
                "SELECT id, payload FROM operations WHERE kind = 'file_op' AND status = 'pending'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            for (id, payload) in rows {
                sqlx::query(
                    "UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1",
                )
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
                let Operation::FileOp {
                    tenant_id, request, ..
                } = serde_json::from_value(payload).unwrap()
                else {
                    panic!("not a file op")
                };
                tokio::time::sleep(delay).await;
                let dir = base.join(tenant_id.to_string());
                std::fs::create_dir_all(&dir).unwrap();
                let root = builtpanel_safe_fs::Root::open(&dir).unwrap();
                let result = builtpanel_fileops::handle(&root, request);
                let output = serde_json::to_value(Output::File { result }).unwrap();
                builtpanel_database::operations::succeed(&pool, id, &output)
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bp-files-api-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_file_manager_works_on_the_account_own_files_through_the_node(pool: PgPool) {
    let base = scratch("main");
    let e = env(&pool, Duration::from_secs(20)).await;
    let (alice, _) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let node = node(pool.clone(), base.clone(), Duration::ZERO);
    let f = "/api/v1/client/files";

    // Write, read, edit with the conflict check, and the editor's size limit.
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{f}/content"),
            Some(json!({"path": "index.php", "content": "<?php echo 1;"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let (status, got) = e
        .call(&a, "GET", &format!("{f}/content?path=index.php"), None)
        .await;
    assert_eq!(
        (status, got["content"].as_str(), got["binary"].as_bool()),
        (StatusCode::OK, Some("<?php echo 1;"), Some(false))
    );
    let mtime = got["mtime"].as_i64().unwrap();
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{f}/content"),
            Some(json!({"path": "index.php", "content": "x", "expected_mtime": mtime - 50})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "changed_meanwhile")
    );
    let (status, _) = e
        .call(
            &a,
            "PUT",
            &format!("{f}/content"),
            Some(json!({"path": "index.php", "content": "<?php echo 2;", "expected_mtime": mtime})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{f}/content"),
            Some(json!({"path": "index.php", "content": "n", "create_only": true})),
        )
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "exists"));
    e.set(&pool, json!({"edit_size_kb": 1})).await;
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{f}/content"),
            Some(json!({"path": "big.txt", "content": "x".repeat(1025)})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
        "{got}"
    );
    std::fs::write(
        base.join(
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .to_string(),
        )
        .join("big.bin"),
        vec![b'a'; 4000],
    )
    .unwrap();
    let (status, got) = e
        .call(&a, "GET", &format!("{f}/content?path=big.bin"), None)
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
        "{got}"
    );

    // Folders, listing and paging.
    for n in ["a", "b", "c"] {
        let (status, _) = e
            .call(
                &a,
                "POST",
                &format!("{f}/folders"),
                Some(json!({"path": n})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, got) = e
        .call(&a, "GET", &format!("{f}?path=&limit=10"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let names: Vec<&str> = got["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["a", "b", "c", "big.bin", "index.php"],
        "folders first"
    );
    assert_eq!(
        (
            got["total"].clone(),
            got["offset"].clone(),
            got["limit"].clone()
        ),
        (json!(5), json!(0), json!(10))
    );
    e.set(&pool, json!({"page_size": 10})).await;
    let (_, got) = e
        .call(&a, "GET", &format!("{f}?path=&limit=500"), None)
        .await;
    assert_eq!(got["limit"], 10, "the page size setting is the most");
    let (_, got) = e
        .call(&a, "GET", &format!("{f}?path=&offset=3&limit=10"), None)
        .await;
    assert_eq!(got["items"].as_array().unwrap().len(), 2);

    // Nobody else sees them, nobody leaves the folder.
    let (_, got) = e.call(&b, "GET", &format!("{f}?path="), None).await;
    assert_eq!(got["total"], 0);
    for path in ["../bobby-files", "/etc", "a/../../x", ".trash"] {
        let (status, got) = e
            .call(
                &a,
                "GET",
                &format!("{f}?path={}", path.replace('/', "%2F")),
                None,
            )
            .await;
        assert_eq!(
            (status, code(&got)),
            (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
            "{path}: {got}"
        );
    }
    let (status, _) = e
        .call(&a, "GET", &format!("{f}/stat?path=nothing"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Move, copy (never over something), delete to the trash, restore.
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{f}/move"),
            Some(json!({"from": "index.php", "to": "a/index.php"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/copy"),
            Some(json!({"from": "a", "to": "b"})),
        )
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "exists"));
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/copy"),
            Some(json!({"from": "a", "to": "c/a-copy"})),
        )
        .await;
    assert_eq!((status, got["count"].clone()), (StatusCode::OK, json!(2)));
    let (status, _) = e.call(&a, "DELETE", &format!("{f}?path=a"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, trash) = e.call(&a, "GET", &format!("{f}/trash"), None).await;
    assert_eq!(trash["items"][0]["path"], "a");
    let id = trash["items"][0]["id"].as_str().unwrap().to_owned();
    let (status, _) = e
        .call(&a, "POST", &format!("{f}/trash/{id}/restore"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, got) = e
        .call(&a, "GET", &format!("{f}/stat?path=a%2Findex.php"), None)
        .await;
    assert_eq!(got["kind"], "file");
    let (status, got) = e.call(&a, "DELETE", &format!("{f}/trash"), None).await;
    assert_eq!((status, got["count"].clone()), (StatusCode::OK, json!(0)));

    // Permissions: setuid is refused.
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/permissions"),
            Some(json!({"path": "a/index.php", "mode": 0o4755})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
        "{got}"
    );
    assert_eq!(got["errors"][0]["code"], "invalid_mode");
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{f}/permissions"),
            Some(json!({"path": "a/index.php", "mode": 0o640})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Packing and unpacking; a hostile archive is refused.
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/archive"),
            Some(json!({"paths": ["a"], "dest": "a.zip", "format": "zip"})),
        )
        .await;
    assert_eq!(
        (status, got["count"].clone()),
        (StatusCode::OK, json!(2)),
        "{got}"
    );
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{f}/archive"),
            Some(json!({"paths": ["a"], "dest": "a2.zip", "format": "rar"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/extract"),
            Some(json!({"path": "a.zip", "dest": "unpacked"})),
        )
        .await;
    assert_eq!(
        (status, got["count"].clone()),
        (StatusCode::OK, json!(2)),
        "{got}"
    );
    let tenant_dir = base.join(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .to_string(),
    );
    std::fs::write(tenant_dir.join("evil.zip"), {
        let mut zip = zip_with_name("../../etc/passwd");
        zip.shrink_to_fit();
        zip
    })
    .unwrap();
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/extract"),
            Some(json!({"path": "evil.zip", "dest": "evil"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    assert_eq!(got["errors"][0]["code"], "bad_archive");

    // What was changed is in the audit log; reading and listing are not.
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_logs WHERE action LIKE 'files.%' ORDER BY at")
            .fetch_all(&pool)
            .await
            .unwrap();
    for wanted in [
        "files.written",
        "files.folder_made",
        "files.moved",
        "files.copied",
        "files.deleted",
        "files.restored",
        "files.permissions_changed",
        "files.archived",
        "files.extracted",
    ] {
        assert!(
            actions.iter().any(|a| a == wanted),
            "{wanted} in {actions:?}"
        );
    }
    assert!(
        !actions
            .iter()
            .any(|a| a.contains("read") || a.contains("list"))
    );
    node.abort();
    let _ = std::fs::remove_dir_all(&base);
}

fn zip_with_name(name: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(name, zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"pwn").unwrap();
    zip.finish().unwrap().into_inner()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn slow_work_answers_202_and_the_answer_is_asked_for_later_by_its_owner_only(pool: PgPool) {
    let base = scratch("slow");
    let e = env(&pool, Duration::from_millis(200)).await;
    let (alice, _) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let node = node(pool.clone(), base.clone(), Duration::from_millis(900));
    let (status, got) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/files/folders",
            Some(json!({"path": "slow"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{got}");
    let id = got["operation_id"].as_str().unwrap().to_owned();
    let url = format!("/api/v1/client/files/operations/{id}");
    let (status, _) = e.call(&b, "GET", &url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "someone else's operation");
    let mut last = (StatusCode::ACCEPTED, Value::Null);
    for _ in 0..40 {
        last = e.call(&a, "GET", &url, None).await;
        if last.0 != StatusCode::ACCEPTED {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        (last.0, last.1["count"].clone()),
        (StatusCode::OK, json!(1)),
        "{:?}",
        last.1
    );
    // A plan without the feature has no file manager.
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['domains']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, got) = e.call(&a, "GET", "/api/v1/client/files?path=", None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
    node.abort();
    let _ = std::fs::remove_dir_all(&base);
}

fn sha(data: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn big_files_go_up_in_chunks_that_can_be_resumed_and_come_down_through_a_five_minute_link(
    pool: PgPool,
) {
    let e = env(&pool, Duration::from_secs(20)).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let f = "/api/v1/client/files";
    let data: Vec<u8> = (0..30_000u32).flat_map(|i| i.to_le_bytes()).collect();

    // What cannot start.
    for (body, why) in [
        (json!({"path": "../x", "size_bytes": 10}), "invalid_path"),
        (
            json!({"path": ".trash/x", "size_bytes": 10}),
            "invalid_path",
        ),
        (json!({"path": "a.bin", "size_bytes": 0}), "empty_file"),
        (
            json!({"path": "a.bin", "size_bytes": 10, "sha256": "zz"}),
            "invalid_checksum",
        ),
    ] {
        let (status, got) = e
            .call(&a, "POST", &format!("{f}/uploads"), Some(body))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {got}");
        assert!(got.to_string().contains(why), "{why}: {got}");
    }
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("{f}/uploads"),
            Some(json!({"path": "a.bin", "size_bytes": 5_000_000_000_u64})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_large")
    );

    // A real upload, with a break in the middle: ask where it stands, go on from there.
    let (status, up) = e
        .call(
            &a,
            "POST",
            &format!("{f}/uploads"),
            Some(json!({"path": "www/big/a.bin", "size_bytes": data.len(), "sha256": sha(&data)})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{up}");
    let id = up["id"].as_str().unwrap().to_owned();
    let chunk = |n: i32| format!("{f}/uploads/{id}/chunks/{n}");
    let (status, _, _) = e
        .raw(Some(&b), "PUT", &chunk(0), &[], data[..1000].to_vec())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not yours");
    let (status, _, _) = e
        .raw(Some(&a), "PUT", &chunk(1), &[], data[..1000].to_vec())
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, body) = e
        .raw(Some(&a), "PUT", &chunk(0), &[], data[..50_000].to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    // "The tab was closed": the new page asks, and goes on.
    let (_, now) = e
        .call(&a, "GET", &format!("{f}/transfers/{id}"), None)
        .await;
    assert_eq!(
        (now["received_bytes"].clone(), now["next_chunk"].clone()),
        (json!(50_000), json!(1))
    );
    let (status, got) = e
        .call(&a, "POST", &format!("{f}/uploads/{id}/complete"), None)
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "incomplete"));
    let (status, _, _) = e
        .raw(Some(&a), "PUT", &chunk(1), &[], data[50_000..].to_vec())
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, done) = e
        .call(&a, "POST", &format!("{f}/uploads/{id}/complete"), None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{done}");
    assert_eq!(done["status"], "queued");

    // The node's side: a token only for it, ranges work, never in the ledger.
    let uuid: Uuid = id.parse().unwrap();
    let (op, payload): (Uuid, Value) =
        sqlx::query_as("SELECT id, payload FROM operations WHERE kind = 'fetch_file'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["dest"], "www/big/a.bin");
    assert!(payload.get("token").is_none_or(Value::is_null), "{payload}");
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_token FROM file_transfers WHERE id = $1")
            .bind(uuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    let token = String::from_utf8(
        MasterKey::from_bytes(&KEY)
            .open(
                &builtpanel_database::file_transfers::token_context(uuid),
                &sealed,
            )
            .unwrap(),
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    let data_url = format!("/api/v1/public/files/transfers/{id}/data");
    let (status, _, _) = e.raw(None, "GET", &data_url, &[], Vec::new()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, body) = e
        .raw(
            None,
            "GET",
            &data_url,
            &[("authorization", &bearer), ("accept-encoding", "gzip")],
            Vec::new(),
        )
        .await;
    assert_eq!((status, body == data), (StatusCode::OK, true));
    let (status, _, body) = e
        .raw(
            None,
            "GET",
            &data_url,
            &[("authorization", &bearer), ("range", "bytes=100000-")],
            Vec::new(),
        )
        .await;
    assert_eq!(
        (status, body),
        (StatusCode::PARTIAL_CONTENT, data[100_000..].to_vec())
    );
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    let (_, now) = e
        .call(&a, "GET", &format!("{f}/transfers/{id}"), None)
        .await;
    assert_eq!(now["status"], "running");
    builtpanel_database::operations::succeed(
        &pool,
        op,
        &json!({"type": "file_fetched", "size_bytes": data.len()}),
    )
    .await
    .unwrap();
    let (_, now) = e
        .call(&a, "GET", &format!("{f}/transfers/{id}"), None)
        .await;
    assert_eq!(now["status"], "done", "{now}");
    let (status, _, _) = e
        .raw(
            None,
            "GET",
            &data_url,
            &[("authorization", &bearer)],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no more after the end");
    let spooled = spool_dir().join(format!("{}.upload", id.replace('-', "")));
    assert!(spooled.exists());
    assert_eq!(
        builtpanel_mod_files::transfers::sweep(&pool, &spool_dir())
            .await
            .unwrap(),
        1
    );
    assert!(
        !spooled.exists(),
        "the spooled file is removed once the node has it"
    );

    // A file that does not match its checksum ends the transfer at once.
    let (_, up) = e
        .call(
            &a,
            "POST",
            &format!("{f}/uploads"),
            Some(json!({"path": "x.bin", "size_bytes": 4, "sha256": sha(b"nope")})),
        )
        .await;
    let bad = up["id"].as_str().unwrap().to_owned();
    let (status, _, _) = e
        .raw(
            Some(&a),
            "PUT",
            &format!("{f}/uploads/{bad}/chunks/0"),
            &[],
            b"data".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, got) = e
        .call(&a, "POST", &format!("{f}/uploads/{bad}/complete"), None)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    // An upload can be given up.
    let (_, up) = e
        .call(
            &a,
            "POST",
            &format!("{f}/uploads"),
            Some(json!({"path": "y.bin", "size_bytes": 4})),
        )
        .await;
    let giveup = up["id"].as_str().unwrap().to_owned();
    let (status, _) = e
        .call(&a, "DELETE", &format!("{f}/transfers/{giveup}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A download: the node sends the file, the client gets a five-minute link.
    let (status, dl) = e
        .call(
            &a,
            "POST",
            &format!("{f}/downloads"),
            Some(json!({"path": "www/big/a.bin"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{dl}");
    let did = dl["id"].as_str().unwrap().to_owned();
    let duuid: Uuid = did.parse().unwrap();
    let (op, payload): (Uuid, Value) =
        sqlx::query_as("SELECT id, payload FROM operations WHERE kind = 'send_file'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        payload["url"],
        format!("https://panel.example.com:8443/api/v1/public/files/transfers/{did}")
    );
    assert!(payload.get("token").is_none_or(Value::is_null));
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_token FROM file_transfers WHERE id = $1")
            .bind(duuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    let dtoken = String::from_utf8(
        MasterKey::from_bytes(&KEY)
            .open(
                &builtpanel_database::file_transfers::token_context(duuid),
                &sealed,
            )
            .unwrap(),
    )
    .unwrap();
    let dbearer = format!("Bearer {dtoken}");
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    let up_url = |n: i32| format!("/api/v1/public/files/transfers/{did}/chunks/{n}");
    let (status, _, _) = e
        .raw(None, "PUT", &up_url(0), &[], data[..100].to_vec())
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &up_url(0),
            &[("authorization", &dbearer)],
            data[..70_000].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &up_url(1),
            &[("authorization", &dbearer)],
            data[70_000..].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, got) = e
        .call(&a, "POST", &format!("{f}/transfers/{did}/link"), None)
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "not_ready"));
    let done_url = format!("/api/v1/public/files/transfers/{did}/complete");
    let (status, _, _) = e
        .raw(
            None,
            "POST",
            &done_url,
            &[
                ("authorization", &dbearer),
                ("content-type", "application/json"),
            ],
            json!({"size_bytes": data.len(), "sha256": sha(&data)})
                .to_string()
                .into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    builtpanel_database::operations::succeed(
        &pool,
        op,
        &json!({"type": "file_sent", "size_bytes": data.len()}),
    )
    .await
    .unwrap();
    let (_, now) = e
        .call(&a, "GET", &format!("{f}/transfers/{did}"), None)
        .await;
    assert_eq!(
        (now["status"].clone(), now["size_bytes"].clone()),
        (json!("ready"), json!(data.len())),
        "{now}"
    );
    let (status, _) = e
        .call(&b, "POST", &format!("{f}/transfers/{did}/link"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, link) = e
        .call(&a, "POST", &format!("{f}/transfers/{did}/link"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{link}");
    let url = link["url"].as_str().unwrap().to_owned();
    let (status, headers, body) = e
        .raw(
            None,
            "GET",
            &url,
            &[("accept-encoding", "gzip")],
            Vec::new(),
        )
        .await;
    assert_eq!((status, body == data), (StatusCode::OK, true));
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"a.bin\""
    );
    let (status, _, body) = e
        .raw(None, "GET", &url, &[("range", "bytes=1000-")], Vec::new())
        .await;
    assert_eq!(
        (status, body),
        (StatusCode::PARTIAL_CONTENT, data[1000..].to_vec())
    );
    let (status, _, _) = e
        .raw(
            None,
            "GET",
            &url.replace("token=", "token=x"),
            &[],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    sqlx::query(
        "UPDATE file_transfers SET link_expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(duuid)
    .execute(&pool)
    .await
    .unwrap();
    let (status, _, _) = e.raw(None, "GET", &url, &[], Vec::new()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the link has run out");
    let (status, link) = e
        .call(&a, "POST", &format!("{f}/transfers/{did}/link"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a new link can be asked for: {link}"
    );
    let _ = tenant;
    let _ = std::fs::remove_dir_all(spool_dir());
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_disk_usage_is_measured_by_the_node_and_kept_for_the_client(pool: PgPool) {
    let base = scratch("usage");
    let e = env(&pool, Duration::from_secs(20)).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice).await;
    let (status, got) = e.call(&a, "GET", "/api/v1/client/files/usage", None).await;
    assert_eq!(
        (
            status,
            got["measured_at"].clone(),
            got["total_bytes"].clone()
        ),
        (StatusCode::OK, Value::Null, json!(0))
    );
    // The panel queues a measurement for every active account, once per slot.
    assert_eq!(builtpanel_mod_files::measure_usage(&pool).await.unwrap(), 1);
    assert_eq!(
        builtpanel_mod_files::measure_usage(&pool).await.unwrap(),
        0,
        "once per six hours"
    );
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'file_op'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["request"]["type"], "disk_usage");
    // The node measures (the real walk); what comes back is kept as the dispatcher does.
    let dir = base.join(tenant.to_string());
    std::fs::create_dir_all(dir.join("www/big")).unwrap();
    std::fs::write(dir.join("www/big/a.bin"), vec![1u8; 200_000]).unwrap();
    let root = builtpanel_safe_fs::Root::open(&dir).unwrap();
    let builtpanel_node_protocol::files::FileResult::Usage {
        total_bytes,
        inodes,
        folders,
        partial,
    } = builtpanel_fileops::handle(
        &root,
        builtpanel_node_protocol::files::FileRequest::DiskUsage { top: 5 },
    )
    else {
        panic!()
    };
    builtpanel_database::file_transfers::record_usage(
        &pool,
        tenant,
        i64::try_from(total_bytes).unwrap(),
        i64::try_from(inodes).unwrap(),
        partial,
        &serde_json::to_value(&folders).unwrap(),
    )
    .await
    .unwrap();
    let (_, got) = e.call(&a, "GET", "/api/v1/client/files/usage", None).await;
    assert!(got["measured_at"].is_string());
    assert!(got["total_bytes"].as_i64().unwrap() >= 200_000);
    assert_eq!(got["inodes"], 3);
    assert_eq!(got["folders"][0]["path"], "www");
    // "Measure now" is allowed and queues one (again within five minutes: the same one).
    let (status, one) = e.call(&a, "POST", "/api/v1/client/files/usage", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{one}");
    let (_, two) = e.call(&a, "POST", "/api/v1/client/files/usage", None).await;
    assert_eq!(one["operation_id"], two["operation_id"]);
    let _ = std::fs::remove_dir_all(&base);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn ftp_accounts_are_made_on_the_node_in_the_background_and_passwords_never_stay(
    pool: PgPool,
) {
    let e = env(&pool, Duration::from_secs(20)).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let path = "/api/v1/client/ftp-accounts";
    sqlx::query("UPDATE plans SET limits = '{\"max_ftp_accounts\": 2}' WHERE id = (SELECT plan_id FROM tenants WHERE id = $1)")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();

    let (status, list) = e.call(&a, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        (
            list["prefix"].clone(),
            list["limit"].clone(),
            list["plain_allowed"].clone()
        ),
        (json!("alice_"), json!(2), json!(false))
    );
    assert_eq!(
        (list["sftp_port"].clone(), list["ftp_port"].clone()),
        (json!(2222), json!(2121))
    );

    // A password is made and shown once; the node gets it sealed, never in the ledger.
    let (status, made) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"name": "site", "home": "www"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(
        (
            made["account"]["name"].clone(),
            made["account"]["status"].clone()
        ),
        (json!("alice_site"), json!("pending"))
    );
    let password = made["password"].as_str().unwrap().to_owned();
    let id: Uuid = made["account"]["id"].as_str().unwrap().parse().unwrap();
    let (op, payload): (Uuid, Value) =
        sqlx::query_as("SELECT id, payload FROM operations WHERE kind = 'upsert_ftp_account'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (
            payload["name"].clone(),
            payload["home"].clone(),
            payload["uid"].clone()
        ),
        (json!("alice_site"), json!("www"), json!(10_000))
    );
    assert_eq!(
        (
            payload["sftp"].clone(),
            payload["ftps"].clone(),
            payload["ftp_plain"].clone()
        ),
        (json!(true), json!(true), json!(false))
    );
    assert!(payload.get("password").is_none_or(Value::is_null));
    let in_ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE payload::text LIKE '%' || $1 || '%'",
    )
    .bind(&password)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(in_ledger, 0);
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed FROM ftp_account_passwords WHERE account_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let opened = MasterKey::from_bytes(&KEY)
        .open(
            &builtpanel_database::ftp_accounts::password_context(id),
            &sealed,
        )
        .unwrap();
    assert_eq!(opened, password.as_bytes());
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::operations::succeed(
        &pool,
        op,
        &json!({"type": "ftp_account_set", "created": true}),
    )
    .await
    .unwrap();
    let (_, list) = e.call(&a, "GET", path, None).await;
    assert_eq!(list["items"][0]["status"], "ready");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM ftp_account_passwords")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "the password goes when the node has it");

    // What cannot be.
    for (body, why) in [
        (json!({"name": "Bad Name"}), "invalid_name"),
        (json!({"name": "x", "home": "../b"}), "invalid_path"),
        (json!({"name": "x", "home": ".trash"}), "invalid_path"),
        (
            json!({"name": "x", "ftp_plain": true}),
            "plain_ftp_disabled",
        ),
        (
            json!({"name": "x", "sftp": false, "ftps": false}),
            "no_protocol",
        ),
        (json!({"name": "x", "password": "weak"}), "too_short"),
    ] {
        let (status, got) = e.call(&a, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {got}");
        assert!(
            got.to_string().contains(why) || why == "too_short",
            "{why}: {got}"
        );
    }
    let (status, got) = e
        .call(&a, "POST", path, Some(json!({"name": "alice_site"})))
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "name_taken"));
    let (status, _) = e
        .call(&a, "POST", path, Some(json!({"name": "second"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, got) = e
        .call(&a, "POST", path, Some(json!({"name": "third"})))
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "limit_reached")
    );

    // Changing: the node is told the new settings, not the password.
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{id}"),
            Some(json!({"read_only": true, "quota_mb": 50, "home": "www/sub"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(
        (
            got["read_only"].clone(),
            got["quota_mb"].clone(),
            got["status"].clone()
        ),
        (json!(true), json!(50), json!("pending"))
    );
    let last: Value = sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'upsert_ftp_account' ORDER BY created_at DESC LIMIT 1").fetch_one(&pool).await.unwrap();
    assert_eq!(
        (
            last["read_only"].clone(),
            last["quota_mb"].clone(),
            last["home"].clone()
        ),
        (json!(true), json!(50), json!("www/sub"))
    );
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{id}"),
            Some(json!({"ftp_plain": true})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    // A new password.
    let (status, got) = e
        .call(&a, "PUT", &format!("{path}/{id}/password"), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert!(got["password"].is_string());
    // Others cannot touch it.
    for (m, p) in [
        ("PUT", format!("{path}/{id}")),
        ("DELETE", format!("{path}/{id}")),
        ("PUT", format!("{path}/{id}/password")),
    ] {
        let (status, _) = e.call(&b, m, &p, Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{m} {p}");
    }
    let (_, theirs) = e.call(&b, "GET", path, None).await;
    assert_eq!(theirs["items"].as_array().unwrap().len(), 0);
    // Removing.
    let (status, _) = e.call(&a, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let drop_op: Uuid =
        sqlx::query_scalar("SELECT id FROM operations WHERE kind = 'drop_ftp_account'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(drop_op)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::operations::succeed(
        &pool,
        drop_op,
        &json!({"type": "ftp_account_dropped", "existed": true}),
    )
    .await
    .unwrap();
    let (_, list) = e.call(&a, "GET", path, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");
    // A plan without the feature has none.
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, got) = e.call(&a, "GET", path, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
}
