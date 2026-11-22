//! The panel's automatic update against a real database (T19.12): it tells the administrators once,
//! installs by itself only when the settings and the clock say so, and tells them when that failed.
//!
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use builtpanel_mod_system::update::{UpdateConfig, UpdateOffer, auto_round};
use builtpanel_settings::{Scope, SettingsRegistry};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn offer(version: &str) -> UpdateOffer {
    UpdateOffer {
        version: version.to_owned(),
        release_date: "2026-11-20T10:00:00Z".to_owned(),
        channel: "stable".to_owned(),
        ed25519_signature: "sig".to_owned(),
        key_id: "release-1".to_owned(),
        checksum_sha256: String::new(),
        changelog: Vec::new(),
        required_migrations: Vec::new(),
        can_rollback: true,
    }
}

async fn events(pool: &PgPool) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox WHERE event_type = 'panel_update' ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    rows.into_iter().map(|(p,)| p).collect()
}

fn stages(events: &[Value]) -> Vec<(&str, &str)> {
    events
        .iter()
        .map(|e| (e["version"].as_str().unwrap(), e["stage"].as_str().unwrap()))
        .collect()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_panel_tells_once_and_updates_itself_only_when_allowed(pool: PgPool) {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../../modules/system/settings.schema.json"
    ))
    .unwrap();
    let registry = SettingsRegistry::build(vec![("system".to_owned(), schema)]).unwrap();
    let root: PathBuf = std::env::temp_dir().join(format!("bp-update-{}", Uuid::now_v7()));
    let config = UpdateConfig {
        repo: "owner/name".to_owned(),
        request_dir: root.join("request"),
        status_dir: root.join("status"),
    };
    std::fs::create_dir_all(&config.request_dir).unwrap();
    std::fs::create_dir_all(&config.status_dir).unwrap();
    let request = config.request_dir.join("request");
    let day = 86_400_i64;
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        / day
        * day;
    let in_window = today + 4 * 3600; // 04:00 UTC, the window opens at 03:00
    let outside = today + 12 * 3600;

    // A release appears: the administrators are told, with the days left; nothing is installed.
    let v15 = offer("0.1.15");
    auto_round(&pool, &registry, &config, in_window, Some(&v15))
        .await
        .unwrap();
    let told = events(&pool).await;
    assert_eq!(stages(&told), [("0.1.15", "available")]);
    assert!(
        told[0]["auto_in_days"]
            .as_i64()
            .is_some_and(|d| (1..=4).contains(&d)),
        "the default waits three days: {told:?}"
    );
    assert!(!request.exists());
    // Looking again changes nothing.
    auto_round(&pool, &registry, &config, in_window, Some(&v15))
        .await
        .unwrap();
    assert_eq!(events(&pool).await.len(), 1, "told once");

    // Four days on, but outside the update hours: still nothing.
    sqlx::query("UPDATE panel_updates SET first_seen_at = now() - interval '4 days'")
        .execute(&pool)
        .await
        .unwrap();
    auto_round(&pool, &registry, &config, outside, Some(&v15))
        .await
        .unwrap();
    assert!(!request.exists(), "not in the update hours");

    // In the hours it asks the update service, and says so, once.
    auto_round(&pool, &registry, &config, in_window, Some(&v15))
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&request).unwrap(), "0.1.15\n");
    assert_eq!(
        stages(&events(&pool).await),
        [("0.1.15", "available"), ("0.1.15", "started")]
    );
    std::fs::remove_file(&request).unwrap();
    auto_round(&pool, &registry, &config, in_window, Some(&v15))
        .await
        .unwrap();
    assert!(!request.exists(), "one attempt for a release");

    // It did not work: the administrators are told, once.
    std::fs::write(
        config.status_dir.join("status.json"),
        json!({"state": "rolled_back", "step": "rolled_back", "version": "0.1.15",
               "from_version": "0.1.14", "message": "did not start", "updated_at": "2026-11-23T03:10:00Z"})
        .to_string(),
    )
    .unwrap();
    for _ in 0..2 {
        auto_round(&pool, &registry, &config, in_window, Some(&v15))
            .await
            .unwrap();
    }
    assert_eq!(
        stages(&events(&pool).await),
        [
            ("0.1.15", "available"),
            ("0.1.15", "started"),
            ("0.1.15", "failed")
        ]
    );

    // Switched off: the next release is announced but never installed by itself.
    let mut conn = pool.acquire().await.unwrap();
    let values = json!({"auto_update": false});
    builtpanel_settings::set(
        &registry,
        &mut conn,
        "system",
        Scope::Global,
        values.as_object().unwrap(),
        None,
    )
    .await
    .unwrap();
    drop(conn);
    let v16 = offer("0.1.16");
    auto_round(&pool, &registry, &config, in_window, Some(&v16))
        .await
        .unwrap();
    sqlx::query("UPDATE panel_updates SET first_seen_at = now() - interval '9 days' WHERE version = '0.1.16'")
        .execute(&pool)
        .await
        .unwrap();
    auto_round(&pool, &registry, &config, in_window, Some(&v16))
        .await
        .unwrap();
    assert!(!request.exists(), "automatic updates are off");
    let told = events(&pool).await;
    let announced = told.last().unwrap();
    assert_eq!(announced["version"], "0.1.16");
    assert!(
        told.iter().filter(|e| e["version"] == "0.1.16").count() == 1,
        "only the announcement"
    );
    let _ = std::fs::remove_dir_all(&root);
}
