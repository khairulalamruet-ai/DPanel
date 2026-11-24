//! The modules table and its sync (T03.03).
//! Ignored by default; see tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::modules::{ModuleRecord, SyncReport, enabled_names, list, sync};
use serde_json::{Value, json};
use sqlx::PgPool;

fn record<'a>(
    name: &'a str,
    version: &'a str,
    core: bool,
    manifest: &'a Value,
) -> ModuleRecord<'a> {
    ModuleRecord {
        name,
        version,
        category: if core { "core" } else { "web" },
        core,
        manifest,
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_new_module_appears_by_itself_and_a_removed_one_is_marked_missing(pool: PgPool) {
    let (m1, m2) = (
        json!({"module": {"name": "auth"}}),
        json!({"module": {"name": "domains"}}),
    );
    let first = sync(&pool, &[record("auth", "0.1.0", true, &m1)])
        .await
        .unwrap();
    assert_eq!(first.added, ["auth"]);
    assert!(first.missing.is_empty() && first.updated.is_empty());

    // The build gains a module: it shows up, switched on, without anyone touching the database.
    let second = sync(
        &pool,
        &[
            record("auth", "0.1.0", true, &m1),
            record("domains", "1.0.0", false, &m2),
        ],
    )
    .await
    .unwrap();
    assert_eq!(second.added, ["domains"]);
    assert_eq!(second.updated, Vec::<String>::new());
    let rows = list(&pool).await.unwrap();
    assert_eq!(rows.len(), 2);
    let domains = rows.iter().find(|r| r.name == "domains").unwrap();
    assert!(domains.enabled && domains.present && !domains.core);
    assert_eq!(domains.manifest, m2);
    assert_eq!(enabled_names(&pool).await.unwrap(), ["auth", "domains"]);

    // Nothing changed: nothing reported.
    let again = sync(
        &pool,
        &[
            record("auth", "0.1.0", true, &m1),
            record("domains", "1.0.0", false, &m2),
        ],
    )
    .await
    .unwrap();
    assert!(again.is_empty(), "{again:?}");

    // The module leaves the build: kept, marked missing, no longer counted as enabled.
    let third = sync(&pool, &[record("auth", "0.1.0", true, &m1)])
        .await
        .unwrap();
    assert_eq!(third.missing, ["domains"]);
    let rows = list(&pool).await.unwrap();
    assert!(!rows.iter().find(|r| r.name == "domains").unwrap().present);
    assert_eq!(enabled_names(&pool).await.unwrap(), ["auth"]);
    // Reported once, not on every start.
    assert!(
        sync(&pool, &[record("auth", "0.1.0", true, &m1)])
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_administrators_choice_survives_upgrades_and_absences(pool: PgPool) {
    let (m, m_new) = (json!({"v": 1}), json!({"v": 2}));
    sync(&pool, &[record("domains", "1.0.0", false, &m)])
        .await
        .unwrap();
    sqlx::query("UPDATE modules SET enabled = false WHERE name = 'domains'")
        .execute(&pool)
        .await
        .unwrap();

    // An upgrade changes version and manifest, not the switch.
    let report = sync(&pool, &[record("domains", "1.1.0", false, &m_new)])
        .await
        .unwrap();
    assert_eq!(report.updated, ["domains"]);
    let row = &list(&pool).await.unwrap()[0];
    assert_eq!(
        (row.version.as_str(), row.enabled, &row.manifest),
        ("1.1.0", false, &m_new)
    );

    // Gone and back: still switched off, reported as restored.
    assert_eq!(sync(&pool, &[]).await.unwrap().missing, ["domains"]);
    let report = sync(&pool, &[record("domains", "1.1.0", false, &m_new)])
        .await
        .unwrap();
    assert_eq!(
        report,
        SyncReport {
            restored: vec!["domains".into()],
            ..SyncReport::default()
        }
    );
    let row = &list(&pool).await.unwrap()[0];
    assert!(row.present && !row.enabled);

    // A module that becomes core is switched on: core cannot be off.
    let report = sync(&pool, &[record("domains", "2.0.0", true, &m_new)])
        .await
        .unwrap();
    assert_eq!(report.updated, ["domains"]);
    let row = &list(&pool).await.unwrap()[0];
    assert!(row.core && row.enabled);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_table_refuses_nonsense_and_concurrent_syncs_agree(pool: PgPool) {
    let m = json!({});
    // A core module cannot be switched off, whoever tries.
    sync(&pool, &[record("auth", "0.1.0", true, &m)])
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE modules SET enabled = false WHERE name = 'auth'")
            .execute(&pool)
            .await
            .is_err()
    );
    for sql in [
        "INSERT INTO modules (name, version, category, core, manifest) VALUES ('Bad Name', '1.0.0', 'web', false, '{}')",
        "INSERT INTO modules (name, version, category, core, manifest) VALUES ('ok', 'one', 'web', false, '{}')",
        "INSERT INTO modules (name, version, category, core, manifest) VALUES ('ok', '1.0.0', 'internet', false, '{}')",
        "INSERT INTO modules (name, version, category, core, manifest) VALUES ('ok', '1.0.0', 'web', false, '[]')",
        "INSERT INTO modules (name, version, category, core, enabled, manifest) VALUES ('ok', '1.0.0', 'core', true, false, '{}')",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }

    // Several control planes starting together: one row per module, no errors.
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            let m = json!({});
            sync(
                &pool,
                &[
                    record("auth", "0.1.0", true, &m),
                    record("dns", "1.0.0", false, &m),
                    record("ssl", "1.0.0", false, &m),
                ],
            )
            .await
        }));
    }
    let mut added = 0;
    for task in tasks {
        added += task.await.unwrap().unwrap().added.len();
    }
    assert_eq!(added, 2, "dns and ssl are added exactly once in total");
    assert_eq!(list(&pool).await.unwrap().len(), 3);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn permission_keys_follow_the_manifests_and_super_admin_gets_them_all(pool: PgPool) {
    use builtpanel_database::{permissions, rbac::load_access};
    let m = json!({"module": {"title": {"en": "Domains"}}});
    sync(&pool, &[record("domains", "1.0.0", false, &m)])
        .await
        .unwrap();

    let first = permissions::sync(
        &pool,
        &[("domains.read", "domains"), ("domains.create", "domains")],
    )
    .await
    .unwrap();
    assert_eq!(first.added, ["domains.read", "domains.create"]);
    let listed = permissions::list(&pool).await.unwrap();
    assert_eq!(
        listed.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        ["domains.create", "domains.read"],
        "by module, then key"
    );
    assert_eq!(listed[0].module_title, Some(json!({"en": "Domains"})));
    assert!(listed.iter().all(|e| e.present));
    assert!(
        permissions::sync(
            &pool,
            &[("domains.read", "domains"), ("domains.create", "domains")]
        )
        .await
        .unwrap()
        .is_empty()
    );

    // A new module's key shows up by itself, and the super administrator holds it
    // without anyone granting it: `*` covers every key, old or new.
    let account: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash) VALUES ('admin', 'root', 'r@example.com', '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, account, "super_admin", None)
        .await
        .unwrap();
    let report = permissions::sync(
        &pool,
        &[
            ("domains.read", "domains"),
            ("domains.create", "domains"),
            ("ssl.issue", "ssl"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(report.added, ["ssl.issue"]);
    let access = load_access(&pool, account).await.unwrap().unwrap();
    let grants = builtpanel_rbac::PermissionSet::new(access.permissions);
    assert!(grants.allows("ssl.issue") && grants.allows("domains.read"));
    // An ordinary administrator does not get it.
    let plain: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash) VALUES ('admin', 'plain', 'p@example.com', '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, plain, "admin", None)
        .await
        .unwrap();
    let access = load_access(&pool, plain).await.unwrap().unwrap();
    assert!(!builtpanel_rbac::PermissionSet::new(access.permissions).allows("ssl.issue"));

    // A key leaves the build: kept and marked, then restored, or moved to another module.
    let report = permissions::sync(&pool, &[("domains.read", "domains"), ("ssl.issue", "ssl")])
        .await
        .unwrap();
    assert_eq!(report.missing, ["domains.create"]);
    assert!(
        !permissions::list(&pool)
            .await
            .unwrap()
            .iter()
            .find(|e| e.key == "domains.create")
            .unwrap()
            .present
    );
    let report = permissions::sync(
        &pool,
        &[
            ("domains.read", "domains"),
            ("domains.create", "websites"),
            ("ssl.issue", "ssl"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(report.restored, ["domains.create"]);
    let entry = permissions::list(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.key == "domains.create")
        .unwrap();
    assert!(entry.present && entry.module == "websites");
    let report = permissions::sync(
        &pool,
        &[
            ("domains.read", "domains"),
            ("domains.create", "domains"),
            ("ssl.issue", "ssl"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(report.moved, ["domains.create"]);

    for bad in ["no_dots", "Domains.read", "a.", "a.b*"] {
        assert!(
            sqlx::query("INSERT INTO permissions (key, module) VALUES ($1, 'domains')")
                .bind(bad)
                .execute(&pool)
                .await
                .is_err(),
            "{bad}"
        );
    }
}
