//! Storing and reading settings on real PostgreSQL.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;

use builtpanel_settings::{Scope, SettingsError, SettingsRegistry, get, set};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn registry() -> SettingsRegistry {
    SettingsRegistry::build(vec![(
        "web".to_owned(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {"type": "string", "minLength": 1, "maxLength": 8, "default": "panel"},
                "workers": {"type": "integer", "minimum": 1, "maximum": 64, "default": 4},
                "node_limit": {"type": "integer", "default": 10, "x-scope": "node"},
                "tenant_note": {"type": "string", "default": "", "x-scope": "tenant"}
            }
        }),
    )])
    .unwrap()
}

fn values(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

async fn read(pool: &PgPool, r: &SettingsRegistry, scope: Scope) -> BTreeMap<String, Value> {
    let mut conn = pool.acquire().await.unwrap();
    get(r, &mut conn, "web", scope).await.unwrap()
}

async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn defaults_overrides_and_a_return_to_the_default(pool: PgPool) {
    let r = registry();
    // Nothing stored: the defaults, only for the scope asked about.
    assert_eq!(
        read(&pool, &r, Scope::Global).await,
        BTreeMap::from([
            ("name".into(), json!("panel")),
            ("workers".into(), json!(4))
        ])
    );

    let mut tx = pool.begin().await.unwrap();
    let change = set(
        &r,
        &mut tx,
        "web",
        Scope::Global,
        &values(json!({"workers": 16, "name": "panel"})),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        change.changed,
        ["workers"],
        "name equals its default: no change"
    );
    assert_eq!(change.before["workers"], json!(4));
    assert_eq!(change.after["workers"], json!(16));
    assert_eq!(read(&pool, &r, Scope::Global).await["workers"], json!(16));
    assert_eq!(rows(&pool).await, 1, "only the real override is stored");

    // Writing the default again removes the override.
    let mut tx = pool.begin().await.unwrap();
    let change = set(
        &r,
        &mut tx,
        "web",
        Scope::Global,
        &values(json!({"workers": 4})),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(change.changed, ["workers"]);
    assert_eq!(rows(&pool).await, 0);
    assert_eq!(read(&pool, &r, Scope::Global).await["workers"], json!(4));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_wrong_type_stores_nothing_even_next_to_good_values(pool: PgPool) {
    let r = registry();
    let mut tx = pool.begin().await.unwrap();
    let err = set(
        &r,
        &mut tx,
        "web",
        Scope::Global,
        &values(json!({"workers": 8, "name": 5})),
        None,
    )
    .await
    .unwrap_err();
    let SettingsError::Invalid(problems) = err else {
        panic!("{err:?}")
    };
    assert_eq!(problems.len(), 1);
    assert_eq!(
        (problems[0].key.as_str(), problems[0].code),
        ("name", "invalid_value")
    );
    tx.commit().await.unwrap();
    assert_eq!(
        rows(&pool).await,
        0,
        "the valid value next to the bad one was not stored either"
    );
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        set(&r, &mut tx, "ghost", Scope::Global, &Map::new(), None).await,
        Err(SettingsError::UnknownModule(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn scopes_do_not_mix_and_each_node_and_tenant_has_its_own(pool: PgPool) {
    let r = registry();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    for (node, value) in [(a, 20), (b, 30)] {
        let mut tx = pool.begin().await.unwrap();
        set(
            &r,
            &mut tx,
            "web",
            Scope::Node(node),
            &values(json!({"node_limit": value})),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    assert_eq!(
        read(&pool, &r, Scope::Node(a)).await["node_limit"],
        json!(20)
    );
    assert_eq!(
        read(&pool, &r, Scope::Node(b)).await["node_limit"],
        json!(30)
    );
    assert_eq!(
        read(&pool, &r, Scope::Node(Uuid::now_v7())).await["node_limit"],
        json!(10)
    );
    assert!(
        !read(&pool, &r, Scope::Global)
            .await
            .contains_key("node_limit")
    );

    // A tenant setting cannot be written for a node, nor a global one for a tenant.
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        set(&r, &mut tx, "web", Scope::Node(a), &values(json!({"tenant_note": "x"})), None).await,
        Err(SettingsError::Invalid(p)) if p[0].code == "wrong_scope"
    ));
    assert!(matches!(
        set(&r, &mut tx, "web", Scope::Tenant(a), &values(json!({"workers": 2})), None).await,
        Err(SettingsError::Invalid(p)) if p[0].code == "wrong_scope"
    ));
    set(
        &r,
        &mut tx,
        "web",
        Scope::Tenant(a),
        &values(json!({"tenant_note": "hello"})),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        read(&pool, &r, Scope::Tenant(a)).await["tenant_note"],
        json!("hello")
    );
    assert_eq!(
        read(&pool, &r, Scope::Tenant(b)).await["tenant_note"],
        json!("")
    );
    assert_eq!(
        read(&pool, &r, Scope::Node(a)).await["node_limit"],
        json!(20),
        "same id, other scope: separate"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_rolled_back_change_and_a_removed_schema_property_leave_no_trace(pool: PgPool) {
    let r = registry();
    let mut tx = pool.begin().await.unwrap();
    set(
        &r,
        &mut tx,
        "web",
        Scope::Global,
        &values(json!({"workers": 9})),
        None,
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(read(&pool, &r, Scope::Global).await["workers"], json!(4));

    // A stored value for a setting that the schema no longer has is ignored.
    sqlx::query(
        "INSERT INTO settings (scope, module, key, value) VALUES ('global', 'web', 'retired', '1')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!read(&pool, &r, Scope::Global).await.contains_key("retired"));

    // The table's own checks.
    for sql in [
        "INSERT INTO settings (scope, scope_id, module, key, value) VALUES ('global', gen_random_uuid(), 'web', 'k', '1')",
        "INSERT INTO settings (scope, module, key, value) VALUES ('node', 'web', 'k', '1')",
        "INSERT INTO settings (scope, module, key, value) VALUES ('planet', 'web', 'k', '1')",
        "INSERT INTO settings (scope, module, key, value) VALUES ('global', 'Web', 'k', '1')",
        "INSERT INTO settings (scope, module, key, value) VALUES ('global', 'web', 'Bad Key', '1')",
        "INSERT INTO settings (scope, module, key, value) VALUES ('global', 'web', 'retired', '2')",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
}
