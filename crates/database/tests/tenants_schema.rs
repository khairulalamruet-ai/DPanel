//! Schema tests for T04.05 (tenants).
//! Ignored by default; see tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::tenants::{self, CreateError, NewTenant};
use builtpanel_domain::TenantStatus;
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

async fn account(pool: &PgPool, name: &str, kind: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ($1::account_kind, $2, $2 || '@example.com', $3) RETURNING id",
    )
    .bind(kind)
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn node(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO nodes (name) VALUES ($1) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn plan(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM plans WHERE name = 'basic'")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn make(
    pool: &PgPool,
    owner: Uuid,
    node: Uuid,
    plan: Uuid,
    name: &str,
) -> Result<tenants::Tenant, CreateError> {
    let mut tx = pool.begin().await.unwrap();
    let t = tenants::insert(
        &mut tx,
        &NewTenant {
            owner_account_id: owner,
            node_id: node,
            plan_id: plan,
            username: name,
        },
    )
    .await?;
    tx.commit().await.unwrap();
    Ok(t)
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn ids_are_handed_out_per_node_lowest_first_and_reused(pool: PgPool) {
    let (plan, a, b) = (
        plan(&pool).await,
        node(&pool, "node-a").await,
        node(&pool, "node-b").await,
    );
    let owner = account(&pool, "customer", "client").await;
    let t1 = make(&pool, owner, a, plan, "alice").await.unwrap();
    let t2 = make(&pool, owner, a, plan, "bob").await.unwrap();
    let t3 = make(&pool, owner, b, plan, "carol").await.unwrap();
    assert_eq!(
        (t1.uid, t2.uid, t3.uid),
        (10_000, 10_001, 10_000),
        "each node counts from the start"
    );
    assert_eq!(t1.status, TenantStatus::Provisioning);

    // A tenant is purged (its row removed): its id is free again.
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(t1.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        make(&pool, owner, a, plan, "dave").await.unwrap().uid,
        10_000
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn names_are_unique_and_owners_must_be_clients(pool: PgPool) {
    let (plan, n) = (plan(&pool).await, node(&pool, "node-a").await);
    let owner = account(&pool, "customer", "client").await;
    make(&pool, owner, n, plan, "alice").await.unwrap();
    assert!(matches!(
        make(&pool, owner, n, plan, "alice").await,
        Err(CreateError::UsernameTaken)
    ));
    let other_node = node(&pool, "node-b").await;
    assert!(
        matches!(
            make(&pool, owner, other_node, plan, "alice").await,
            Err(CreateError::UsernameTaken)
        ),
        "unique panel-wide"
    );

    let admin = account(&pool, "someadmin", "admin").await;
    assert!(matches!(
        make(&pool, admin, n, plan, "carol").await,
        Err(CreateError::BadOwner)
    ));
    // An owner that does not exist is not a client account either.
    assert!(matches!(
        make(&pool, Uuid::now_v7(), n, plan, "dave").await,
        Err(CreateError::BadOwner)
    ));
    assert!(matches!(
        make(&pool, owner, Uuid::now_v7(), plan, "erin").await,
        Err(CreateError::UnknownReference)
    ));
    assert!(matches!(
        make(&pool, owner, n, Uuid::now_v7(), "fred").await,
        Err(CreateError::UnknownReference)
    ));
    for sql in [
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid) SELECT a.id, n.id, p.id, 'Bad', 10010 FROM accounts a, nodes n, plans p WHERE a.kind = 'client' AND p.name = 'basic' LIMIT 1",
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid) SELECT a.id, n.id, p.id, 'ok1', 5 FROM accounts a, nodes n, plans p WHERE a.kind = 'client' AND p.name = 'basic' LIMIT 1",
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status) SELECT a.id, n.id, p.id, 'ok2', 10011, 'gone' FROM accounts a, nodes n, plans p WHERE a.kind = 'client' AND p.name = 'basic' LIMIT 1",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn concurrent_creations_on_one_node_never_share_an_id(pool: PgPool) {
    let (plan, n) = (plan(&pool).await, node(&pool, "node-a").await);
    let owner = account(&pool, "customer", "client").await;
    let mut tasks = Vec::new();
    for i in 0..12 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            make(&pool, owner, n, plan, &format!("tenant{i}"))
                .await
                .unwrap()
                .uid
        }));
    }
    let mut uids = Vec::new();
    for t in tasks {
        uids.push(t.await.unwrap());
    }
    uids.sort_unstable();
    assert_eq!(uids, (10_000..10_012).collect::<Vec<i32>>());
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_life_cycle_is_enforced_and_plans_know_their_tenants(pool: PgPool) {
    let (plan, n) = (plan(&pool).await, node(&pool, "node-a").await);
    let owner = account(&pool, "customer", "client").await;
    let t = make(&pool, owner, n, plan, "alice").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert!(
        !tenants::set_status(&mut conn, t.id, TenantStatus::Suspended, 7)
            .await
            .unwrap(),
        "not yet active"
    );
    assert!(
        tenants::set_status(&mut conn, t.id, TenantStatus::Active, 7)
            .await
            .unwrap()
    );
    assert!(
        tenants::set_status(&mut conn, t.id, TenantStatus::Suspended, 7)
            .await
            .unwrap()
    );
    assert!(
        tenants::set_status(&mut conn, t.id, TenantStatus::Active, 7)
            .await
            .unwrap()
    );
    assert_eq!(tenants::counts_by_plan(&pool).await.unwrap(), [(plan, 1)]);

    // A plan with tenants cannot be deleted, a node cannot either.
    assert!(
        sqlx::query("DELETE FROM plans WHERE name = 'basic'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM nodes WHERE name = 'node-a'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM accounts WHERE username = 'customer'")
            .execute(&pool)
            .await
            .is_err()
    );

    // Deleting sets the dates, and the tenant stops counting for its plan.
    assert!(
        tenants::set_status(&mut conn, t.id, TenantStatus::Deleting, 7)
            .await
            .unwrap()
    );
    let (deleted, purge): (bool, bool) = sqlx::query_as(
        "SELECT deleted_at IS NOT NULL, purge_after > now() + interval '6 days' FROM tenants WHERE id = $1",
    )
    .bind(t.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((deleted, purge) == (true, true));
    assert!(tenants::counts_by_plan(&pool).await.unwrap().is_empty());
    assert!(
        !tenants::set_status(&mut conn, t.id, TenantStatus::Active, 7)
            .await
            .unwrap(),
        "deleting is a dead end"
    );
    assert!(
        !tenants::set_status(&mut conn, Uuid::now_v7(), TenantStatus::Active, 7)
            .await
            .unwrap(),
        "unknown tenant"
    );
    // The two columns cannot disagree with the status.
    assert!(
        sqlx::query("UPDATE tenants SET deleted_at = NULL WHERE id = $1")
            .bind(t.id)
            .execute(&pool)
            .await
            .is_err()
    );
}
