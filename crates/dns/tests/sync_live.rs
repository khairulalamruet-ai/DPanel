//! Zone sync against real PostgreSQL, with the in-memory DNS server.
//! Ignored by default; see crates/database/tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use builtpanel_database::dns::{self as store, StoredLine};
use builtpanel_dns::{DnsError, mock::MockProvider, sync::ZoneSync};
use builtpanel_domain::DnsRecord;
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

/// A domain with a zone of its own; returns (tenant, domain, zone).
async fn zone(
    pool: &PgPool,
    tenant: Option<Uuid>,
    name: &str,
    records: &[DnsRecord],
) -> (Uuid, Uuid, Uuid) {
    let tenant = match tenant {
        Some(t) => t,
        None => {
            let account: Uuid = sqlx::query_scalar(
                "INSERT INTO accounts (kind, username, email, password_hash)
                 VALUES ('client', 'alice', 'alice@example.com', $1) RETURNING id",
            )
            .bind(HASH)
            .fetch_one(pool)
            .await
            .unwrap();
            let node: Uuid =
                sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node1') RETURNING id")
                    .fetch_one(pool)
                    .await
                    .unwrap();
            let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
                .fetch_one(pool)
                .await
                .unwrap();
            sqlx::query_scalar(
                "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid)
                 VALUES ($1, $2, $3, 'alice', 10000) RETURNING id",
            )
            .bind(account)
            .bind(node)
            .bind(plan)
            .fetch_one(pool)
            .await
            .unwrap()
        }
    };
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'addon') RETURNING id",
    )
    .bind(tenant)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let z = store::create_zone(&mut conn, domain, name, records)
        .await
        .unwrap();
    (tenant, domain, z.id)
}

fn rec(name: &str, t: &str, content: &str) -> DnsRecord {
    DnsRecord::parse(name, t, content, Some(300), None).unwrap()
}

fn sync(pool: &PgPool, provider: &MockProvider) -> ZoneSync {
    ZoneSync::new(
        pool.clone(),
        Arc::new(provider.clone()),
        Arc::new(vec!["ns1.example.net.".to_owned()]),
    )
}

async fn status(pool: &PgPool, id: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, last_error FROM dns_zones WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_new_zone_is_pushed_once_and_then_left_alone(pool: PgPool) {
    let server = MockProvider::new();
    let (_, _, z) = zone(
        &pool,
        None,
        "example.com",
        &[rec("@", "A", "192.0.2.1"), rec("www", "A", "192.0.2.1")],
    )
    .await;
    assert_eq!(status(&pool, z).await.0, "pending");

    let s = sync(&pool, &server);
    let stats = s.run_once().await;
    assert_eq!((stats.synced, stats.failed), (1, 0));
    assert_eq!(status(&pool, z).await, ("active".to_owned(), None));
    server.with(|st| {
        assert_eq!(st.zones["example.com"].len(), 2);
        assert_eq!(st.nameservers["example.com"], ["ns1.example.net."]);
    });

    // Nothing changed: the next pass does not even call the server.
    let calls = server.with(|st| st.calls.len());
    assert_eq!(s.run_once().await, Default::default());
    assert_eq!(server.with(|st| st.calls.len()), calls);

    // A change makes it pending again; the server then holds the new records.
    sqlx::query("DELETE FROM dns_records WHERE zone_id = $1 AND name = 'www'")
        .bind(z)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE dns_zones SET status = 'pending' WHERE id = $1")
        .bind(z)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(s.run_once().await.synced, 1);
    assert_eq!(
        server.with(|st| st.zones["example.com"].clone()),
        [rec("@", "A", "192.0.2.1")]
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_unreachable_server_leaves_zones_pending_and_a_refusal_marks_them(pool: PgPool) {
    let server = MockProvider::new();
    let (tenant, _, a) = zone(&pool, None, "a.com", &[rec("@", "A", "192.0.2.1")]).await;
    let (_, _, b) = zone(&pool, Some(tenant), "b.com", &[rec("@", "A", "192.0.2.2")]).await;
    let s = sync(&pool, &server);

    // Down: the first zone stays pending with the reason, and the pass stops there.
    server.fail_next(DnsError::Unavailable("connection refused".into()));
    let stats = s.run_once().await;
    assert_eq!((stats.synced, stats.failed), (0, 1));
    let (st, why) = status(&pool, a).await;
    assert_eq!(st, "pending");
    assert!(why.unwrap().contains("connection refused"));
    assert_eq!(
        status(&pool, b).await.0,
        "pending",
        "not even tried: the server is down"
    );
    // Up again: both go through.
    assert_eq!(s.run_once().await.synced, 2);
    assert_eq!(status(&pool, a).await, ("active".to_owned(), None));

    // A refusal is the zone's own problem: it is marked, the other zones still go.
    sqlx::query("UPDATE dns_zones SET status = 'pending'")
        .execute(&pool)
        .await
        .unwrap();
    server.fail_next(DnsError::Rejected("invalid content".into()));
    let stats = s.run_once().await;
    assert_eq!((stats.synced, stats.failed), (1, 1));
    let (st, why) = status(&pool, a).await;
    assert_eq!(st, "error");
    assert!(why.unwrap().contains("invalid content"));
    assert_eq!(status(&pool, b).await.0, "active");
    // Errors wait before they are tried again (15 minutes), unless the zone changes.
    assert_eq!(s.run_once().await, Default::default());
    // The `updated_at` trigger would undo the backdating, so it is off for this one change.
    sqlx::query("ALTER TABLE dns_zones DISABLE TRIGGER dns_zones_set_updated_at")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE dns_zones SET updated_at = now() - interval '20 minutes' WHERE id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE dns_zones ENABLE TRIGGER dns_zones_set_updated_at")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(s.run_once().await.synced, 1);
    assert_eq!(status(&pool, a).await.0, "active");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_edit_made_during_a_push_is_not_lost(pool: PgPool) {
    let server = MockProvider::new();
    let (_, _, z) = zone(&pool, None, "example.com", &[rec("@", "A", "192.0.2.1")]).await;
    let zone_before = store::zone(&pool, z).await.unwrap().unwrap();
    // The zone changes after the push began (its version is the old one).
    sqlx::query("UPDATE dns_zones SET last_error = 'edited' WHERE id = $1")
        .bind(z)
        .execute(&pool)
        .await
        .unwrap();
    let written = store::mark_zone(&pool, z, &zone_before.version, "active", None, None)
        .await
        .unwrap();
    assert!(!written, "a stale push does not mark the zone done");
    assert_eq!(status(&pool, z).await.0, "pending");
    let _ = server;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn zones_deleted_in_the_panel_are_removed_from_the_server(pool: PgPool) {
    let server = MockProvider::new();
    let (tenant, domain, _) = zone(&pool, None, "gone.com", &[rec("@", "A", "192.0.2.1")]).await;
    let (_, keep_domain, _) = zone(
        &pool,
        Some(tenant),
        "stay.com",
        &[rec("@", "A", "192.0.2.2")],
    )
    .await;
    let s = sync(&pool, &server);
    assert_eq!(s.run_once().await.synced, 2);
    assert_eq!(server.with(|st| st.zones.len()), 2);

    // Deleting the domain deletes its zone (a cascade); the trigger queues the removal.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(domain)
        .execute(&pool)
        .await
        .unwrap();
    let queued: Vec<String> = sqlx::query_scalar("SELECT name FROM dns_zone_removals")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(queued, ["gone.com"]);
    let stats = s.run_once().await;
    assert_eq!(stats.removed, 1);
    assert_eq!(
        server.with(|st| st.zones.keys().cloned().collect::<Vec<_>>()),
        ["stay.com"]
    );
    assert_eq!(
        s.run_once().await,
        Default::default(),
        "the list is empty again"
    );

    // Deleted and added back before the job ran: the zone stays.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(keep_domain)
        .execute(&pool)
        .await
        .unwrap();
    let back: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'stay.com', 'addon') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    store::create_zone(&mut conn, back, "stay.com", &[rec("@", "A", "192.0.2.9")])
        .await
        .unwrap();
    drop(conn);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM dns_zone_removals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "creating the zone cancels its removal");
    s.run_once().await;
    assert_eq!(
        server.with(|st| st.zones["stay.com"].clone()),
        [rec("@", "A", "192.0.2.9")]
    );

    // A server that is down keeps the removal on the list.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(back)
        .execute(&pool)
        .await
        .unwrap();
    server.fail_next(DnsError::Unavailable("down".into()));
    assert_eq!(s.run_once().await.removed, 0);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM dns_zone_removals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
    assert_eq!(s.run_once().await.removed, 1);
    assert!(server.with(|st| st.zones.is_empty()));
    let _: Vec<StoredLine> = Vec::new();
}

/// A name server list the test can change.
struct Changing(std::sync::Mutex<Vec<String>>);

impl builtpanel_dns::sync::NameserverSource for Changing {
    fn nameservers(&self) -> builtpanel_dns::BoxFuture<'_, Vec<String>> {
        Box::pin(async move { self.0.lock().unwrap().clone() })
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn changing_the_name_servers_pushes_the_zones_again(pool: PgPool) {
    let server = MockProvider::new();
    let (_, _, z) = zone(&pool, None, "example.com", &[rec("@", "A", "192.0.2.1")]).await;
    let list = Arc::new(Changing(std::sync::Mutex::new(vec![
        "ns1.a.net.".to_owned(),
    ])));
    let s = ZoneSync::new(pool.clone(), Arc::new(server.clone()), list.clone());
    assert_eq!(s.run_once().await.synced, 1);
    assert_eq!(status(&pool, z).await.0, "active");
    assert_eq!(
        s.run_once().await,
        Default::default(),
        "same name servers: nothing to do"
    );
    let calls = server.with(|st| st.calls.len());

    // The administrator changes them: the active zone is pushed again, once.
    *list.0.lock().unwrap() = vec!["ns1.b.net.".to_owned(), "ns2.b.net.".to_owned()];
    assert_eq!(s.run_once().await.synced, 1);
    assert!(server.with(|st| st.calls.len()) > calls);
    assert_eq!(s.run_once().await, Default::default());
    let digest: Option<String> =
        sqlx::query_scalar("SELECT ns_digest FROM dns_zones WHERE id = $1")
            .bind(z)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(digest.as_deref(), Some("ns1.b.net.,ns2.b.net."));

    // A delegation check does not count as a change of the zone.
    let before = store::zone(&pool, z).await.unwrap().unwrap().version;
    store::save_delegation(&pool, z, &serde_json::json!({"status": "ok"}))
        .await
        .unwrap();
    assert_eq!(
        store::zone(&pool, z).await.unwrap().unwrap().version,
        before
    );
    let (saved, _) = store::delegation(&pool, z).await.unwrap().unwrap();
    assert_eq!(saved["status"], "ok");
}
