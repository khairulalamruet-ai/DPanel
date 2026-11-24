//! Usage over time (T10.09): readings become rates, hours are rolled up, old data goes, and
//! thirty days of an account read back fast. Ignored by default; see tests/migrations.rs.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Instant;

use builtpanel_database::usage::{self, Container, Node};
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

async fn tenant(pool: &PgPool, name: &str, uid: i32) -> (Uuid, Uuid) {
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
    (tenant, node)
}

fn container(name: &str, cpu_seconds: u64, rx: u64, memory: u64) -> Container {
    Container {
        name: name.into(),
        memory_bytes: memory,
        cpu_nanos: cpu_seconds * 1_000_000_000,
        pids: 5,
        net_rx: rx,
        net_tx: rx / 2,
        io_read: 0,
        io_write: rx / 4,
    }
}

fn node(busy: u64, total: u64) -> Node {
    Node {
        cpus: 2,
        busy_jiffies: busy,
        total_jiffies: total,
        memory_used: 1 << 30,
        memory_total: 2 << 30,
        swap_used: 0,
        load1_x100: 75,
        disk_used: 10 << 30,
        disk_total: 30 << 30,
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL; see tests/migrations.rs"]
async fn readings_become_rates_per_account_and_for_the_node(pool: PgPool) {
    let (alice, node_id) = tenant(&pool, "alice", 10_000).await;
    let (bob, _) = tenant(&pool, "bob", 10_001).await;
    let t0 = 1_800_000_000; // a whole minute
    // The first reading is only a beginning: levels, no rates.
    usage::record(
        &pool,
        node_id,
        t0,
        &node(1000, 10_000),
        &[
            container("bp-alice-web", 100, 1000, 10 << 20),
            container("bp-alice-php", 200, 5000, 50 << 20),
            container("bp-bob-web", 50, 0, 5 << 20),
            container("bp-db", 999, 999, 999),
        ],
    )
    .await
    .unwrap();
    let first: (i32, i64, i32, i64) = sqlx::query_as(
        "SELECT cpu_millis, memory_bytes, pids, net_rx_bytes FROM usage_minute WHERE tenant_id = $1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first, (0, 60 << 20, 10, 0));
    // A minute later: alice's two containers used 30 s and 6 s of CPU (0.6 CPU), 2000 bytes arrived.
    usage::record(
        &pool,
        node_id,
        t0 + 60,
        &node(1000 + 60, 10_000 + 200),
        &[
            container("bp-alice-web", 130, 2000, 11 << 20),
            container("bp-alice-php", 206, 6000, 51 << 20),
            // Bob's container restarted: its counters are lower. No negative anything.
            container("bp-bob-web", 1, 10, 5 << 20),
            container("bp-db", 1000, 1000, 1000),
        ],
    )
    .await
    .unwrap();
    let second: (i32, i64, i64, i64) = sqlx::query_as(
        "SELECT cpu_millis, memory_bytes, net_rx_bytes, io_write_bytes FROM usage_minute
         WHERE tenant_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(second, (600, 62 << 20, 2000, 500), "{second:?}");
    let bob_now: (i32, i64) = sqlx::query_as(
        "SELECT cpu_millis, net_rx_bytes FROM usage_minute WHERE tenant_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(bob)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bob_now, (0, 0));
    // Only accounts' containers are accounted for (not the database server's).
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_minute")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 4);
    // The node: 60 of 200 jiffies busy.
    let n: (i32, i32, i32) = sqlx::query_as(
        "SELECT cpu_busy_millis, containers, load1_x100 FROM node_usage_minute WHERE node_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, (300, 4, 75));
    // The same minute twice adds up the bytes instead of losing them.
    usage::record(
        &pool,
        node_id,
        t0 + 90,
        &node(1100, 10_400),
        &[
            container("bp-alice-web", 140, 2500, 11 << 20),
            container("bp-alice-php", 207, 6100, 51 << 20),
        ],
    )
    .await
    .unwrap();
    let merged: i64 = sqlx::query_scalar(
        "SELECT net_rx_bytes FROM usage_minute WHERE tenant_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(merged, 2000 + 500 + 100);
    // A container that is gone leaves no counters behind.
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_counters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 2);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL; see tests/migrations.rs"]
async fn minutes_are_rolled_up_into_hours_and_the_old_are_removed(pool: PgPool) {
    let (alice, _) = tenant(&pool, "alice", 10_000).await;
    // One reading every 10 minutes: 3 hours ago, and 50 hours ago.
    for (hours_ago, cpu) in [(3, 100), (3, 300), (50, 999)] {
        sqlx::query(
            "INSERT INTO usage_minute (tenant_id, at, cpu_millis, memory_bytes, pids, net_rx_bytes, net_tx_bytes, io_read_bytes, io_write_bytes)
             VALUES ($1, date_trunc('hour', now()) - make_interval(hours => $2) + make_interval(mins => $3), $4, 1000, 3, 10, 20, 30, 40)",
        )
        .bind(alice)
        .bind(hours_ago)
        .bind(cpu / 100)
        .bind(cpu)
        .execute(&pool)
        .await
        .unwrap();
    }
    let done = usage::maintain(&pool).await.unwrap();
    assert_eq!(done.partitions_made, 3, "{done:?}");
    assert_eq!(done.rolled_up, 2, "{done:?}");
    assert_eq!(done.minutes_removed, 1, "{done:?}");
    let hour: (i32, i64, i64) = sqlx::query_as(
        "SELECT cpu_millis, net_rx_bytes, io_write_bytes FROM usage_hour
         WHERE tenant_id = $1 AND at = date_trunc('hour', now()) - interval '3 hours'",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    // CPU averaged (100 and 300), bytes added up.
    assert_eq!(hour, (200, 20, 80));
    // The second run has nothing new to make.
    let again = usage::maintain(&pool).await.unwrap();
    assert_eq!((again.partitions_made, again.minutes_removed), (0, 0));
    // The 50-hour-old minutes are gone; their hour stays in the hour table.
    assert_eq!(again.rolled_up, 1);
    let hours: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_hour")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hours, 2);
    // A month that is too old is dropped whole.
    sqlx::query("CREATE TABLE usage_hour_2020_01 PARTITION OF usage_hour FOR VALUES FROM ('2020-01-01') TO ('2020-02-01')")
        .execute(&pool)
        .await
        .unwrap();
    let dropped = usage::maintain(&pool).await.unwrap();
    assert_eq!(dropped.partitions_dropped, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL; see tests/migrations.rs"]
async fn thirty_days_of_one_account_read_back_in_under_200_ms_among_many(pool: PgPool) {
    let (alice, _) = tenant(&pool, "alice", 10_000).await;
    usage::maintain(&pool).await.unwrap();
    // 2000 accounts' hours for 30 days: 1.44 million rows.
    sqlx::query(
        "INSERT INTO usage_hour
         SELECT CASE WHEN t = 0 THEN $1 ELSE ('00000000-0000-4000-8000-' || lpad(t::text, 12, '0'))::uuid END,
                date_trunc('hour', now()) - make_interval(hours => h),
                100 + h % 50, 1000000, 4, h, h, h, h
         FROM generate_series(0, 1999) t, generate_series(1, 720) h",
    )
    .bind(alice)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE usage_hour")
        .execute(&pool)
        .await
        .unwrap();
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    // The best of three, to leave out a cold cache.
    let mut best = std::time::Duration::MAX;
    let mut points = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        points = usage::series(&pool, alice, now - 30 * 86_400, now, 3600)
            .await
            .unwrap();
        best = best.min(started.elapsed());
    }
    eprintln!(
        "30 days of one account among 2000: {best:?}, {} points",
        points.len()
    );
    assert!((719..=721).contains(&points.len()), "{}", points.len());
    assert!(best < std::time::Duration::from_millis(200), "{best:?}");
    // Coarser buckets and the recent range (minutes) work too.
    let daily = usage::series(&pool, alice, now - 30 * 86_400, now, 86_400)
        .await
        .unwrap();
    assert!((30..=32).contains(&daily.len()), "{}", daily.len());
    assert!(
        usage::series(&pool, alice, now - 3600, now, 60)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL; see tests/migrations.rs"]
async fn the_fleet_is_shown_as_prometheus_text_and_the_dashboard_uses_what_is_shown(pool: PgPool) {
    let (_, node_id) = tenant(&pool, "alice", 10_000).await;
    let (_, _) = tenant(&pool, "bobby", 10_001).await;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    usage::record(
        &pool,
        node_id,
        now - 60,
        &node(1000, 10_000),
        &[container("bp-alice-web", 10, 0, 1 << 20)],
    )
    .await
    .unwrap();
    usage::record(
        &pool,
        node_id,
        now,
        &node(1100, 10_400),
        &[
            container("bp-alice-web", 40, 500, 2 << 20),
            container("bp-bobby-web", 5, 0, 1 << 20),
        ],
    )
    .await
    .unwrap();
    let text = usage::prometheus_text(&pool).await.unwrap();
    // The newest reading of the node and the accounts, in Prometheus's format.
    assert!(
        text.contains("# TYPE builtpanel_node_cpu_busy_ratio gauge"),
        "{text}"
    );
    assert!(
        text.contains("builtpanel_node_cpu_busy_ratio{node=\"node1\"} 0.25"),
        "{text}"
    );
    assert!(text.contains("builtpanel_node_containers{node=\"node1\"} 2"));
    assert!(text.contains("builtpanel_tenant_memory_bytes{account=\"alice\"} 2097152"));
    assert!(text.contains("builtpanel_tenant_net_receive_bytes{account=\"alice\"} 500"));
    assert!(
        text.contains("builtpanel_tenant_memory_bytes{account=\"bobby\"} 1048576"),
        "{text}"
    );
    assert!(text.contains("builtpanel_tenants{status=\"active\"} 2"));
    // Every metric the Grafana dashboard asks for is among them.
    let dashboard = include_str!("../../../deploy/grafana/builtpanel-fleet.json");
    let mut seen = 0;
    for word in dashboard.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if word.starts_with("builtpanel_") {
            assert!(
                text.contains(&format!("# TYPE {word} gauge")),
                "the dashboard uses {word}, which is not shown"
            );
            seen += 1;
        }
    }
    assert!(seen >= 15, "{seen}");
}
