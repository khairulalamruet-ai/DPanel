//! Schema tests for T05.01 (domains and redirects).
//! Ignored by default; see tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

/// A client with a tenant; returns the tenant id.
async fn tenant(pool: &PgPool, name: &str, uid: i32) -> Uuid {
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
        "INSERT INTO nodes (name) VALUES ($1) ON CONFLICT (name) DO UPDATE SET name = $1 RETURNING id",
    )
    .bind("node1")
    .fetch_one(pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn add(
    pool: &PgPool,
    tenant: Uuid,
    fqdn: &str,
    kind: &str,
    parent: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind, parent_domain_id) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(tenant)
    .bind(fqdn)
    .bind(kind)
    .bind(parent)
    .fetch_one(pool)
    .await
}

fn constraint(err: &sqlx::Error) -> String {
    err.as_database_error()
        .and_then(|e| e.constraint().map(str::to_owned))
        .unwrap_or_else(|| format!("{err}"))
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn names_are_ascii_lower_case_and_unique_panel_wide(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let b = tenant(&pool, "bobby", 10_001).await;
    add(&pool, a, "example.com", "primary", None).await.unwrap();
    // Unique across tenants.
    assert_eq!(
        constraint(
            &add(&pool, b, "example.com", "primary", None)
                .await
                .unwrap_err()
        ),
        "domains_fqdn_key"
    );
    for bad in [
        "Example.com",
        "example",
        "-a.com",
        "a-.com",
        "a..com",
        "a b.com",
        "bücher.de",
        "exa_mple.com",
        "1.2.3.4",
        "example.com.",
        ".example.com",
        "",
    ] {
        assert_eq!(
            constraint(&add(&pool, b, bad, "addon", None).await.unwrap_err()),
            "domains_fqdn_format",
            "{bad:?}"
        );
    }
    assert!(
        add(&pool, b, "xn--bcher-kva.de", "addon", None)
            .await
            .is_ok()
    );
    assert!(
        add(&pool, b, &format!("{}.com", "a".repeat(63)), "addon", None)
            .await
            .is_ok()
    );
    assert!(
        add(&pool, b, &format!("{}.com", "a".repeat(64)), "addon", None)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn one_primary_per_tenant_and_parents_follow_the_kind(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let b = tenant(&pool, "bobby", 10_001).await;
    let main = add(&pool, a, "example.com", "primary", None).await.unwrap();
    assert_eq!(
        constraint(
            &add(&pool, a, "second.com", "primary", None)
                .await
                .unwrap_err()
        ),
        "domains_one_primary_per_tenant"
    );
    assert!(add(&pool, a, "other.org", "addon", None).await.is_ok());
    // sub and alias need a parent; primary and addon must not have one.
    for (name, kind, parent) in [
        ("blog.example.com", "sub", None),
        ("alias.org", "alias", None),
        ("third.com", "addon", Some(main)),
    ] {
        assert_eq!(
            constraint(&add(&pool, a, name, kind, parent).await.unwrap_err()),
            "domains_parent_by_kind",
            "{name}"
        );
    }
    // A subdomain lies below its parent; an alias can be anywhere.
    let blog = add(&pool, a, "blog.example.com", "sub", Some(main))
        .await
        .unwrap();
    assert!(
        add(&pool, a, "deep.blog.example.com", "sub", Some(blog))
            .await
            .is_ok()
    );
    assert!(
        add(&pool, a, "elsewhere.example.net", "alias", Some(main))
            .await
            .is_ok()
    );
    let err = add(&pool, a, "notbelow.org", "sub", Some(main))
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("below its parent"), "{err}");
    let err = add(&pool, a, "xexample.com.evil.org", "sub", Some(main))
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("below its parent"), "{err}");
    // The parent is the tenant's own.
    let err = add(&pool, b, "www.example.com", "sub", Some(main))
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("own tenant"), "{err}");
    // Nothing hangs off an alias.
    let alias: Uuid = sqlx::query_scalar("SELECT id FROM domains WHERE kind = 'alias'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let err = add(&pool, a, "x.example.net", "sub", Some(alias))
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("alias"), "{err}");
    // Deleting a domain takes what hangs off it; deleting the tenant takes everything.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    let left: Vec<String> = sqlx::query_scalar("SELECT fqdn FROM domains ORDER BY fqdn")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["other.org"]);
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM domains")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn docroots_status_and_redirects_are_checked(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let d = add(&pool, a, "example.com", "primary", None).await.unwrap();
    for ok in ["", "public_html", "sites/blog-2/web", "a.b/c_d"] {
        sqlx::query("UPDATE domains SET docroot = $2 WHERE id = $1")
            .bind(d)
            .bind(ok)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{ok:?}: {e}"));
    }
    for bad in [
        "/abs",
        "a/",
        "../x",
        "a/../b",
        ".",
        "a//b",
        "a b",
        "a/b/c/d/e/f/g",
    ] {
        let err = sqlx::query("UPDATE domains SET docroot = $2 WHERE id = $1")
            .bind(d)
            .bind(bad)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(constraint(&err), "domains_docroot_format", "{bad:?}");
    }
    // Suspended exactly when it has a time.
    let err = sqlx::query("UPDATE domains SET status = 'suspended' WHERE id = $1")
        .bind(d)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "domains_suspended_has_time");
    sqlx::query("UPDATE domains SET status = 'suspended', suspended_at = now() WHERE id = $1")
        .bind(d)
        .execute(&pool)
        .await
        .unwrap();

    // Redirects.
    let redirect = |path: &'static str, target: &'static str, code: i16| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO redirects (domain_id, source_path, target, code) VALUES ($1, $2, $3, $4)")
                .bind(d)
                .bind(path)
                .bind(target)
                .bind(code)
                .execute(&pool)
                .await
        }
    };
    redirect("/old", "https://example.org/new", 301)
        .await
        .unwrap();
    redirect("/", "/home", 302).await.unwrap();
    redirect("/a/b.html", "http://x.example/y?z=1#f", 308)
        .await
        .unwrap();
    assert_eq!(
        constraint(&redirect("/old", "/x", 301).await.unwrap_err()),
        "redirects_domain_source_key"
    );
    for bad in ["old", "/a b", "/a?x=1", "/a#f", ""] {
        assert_eq!(
            constraint(&redirect(bad, "/x", 301).await.unwrap_err()),
            "redirects_source_format",
            "{bad:?}"
        );
    }
    for bad in [
        "javascript:alert(1)",
        "ftp://x.org/f",
        "relative/path",
        "https://a b",
        "",
    ] {
        assert_eq!(
            constraint(&redirect("/z", bad, 301).await.unwrap_err()),
            "redirects_target_format",
            "{bad:?}"
        );
    }
    assert_eq!(
        constraint(&redirect("/z", "/x", 300).await.unwrap_err()),
        "redirects_code"
    );
    // They go with their domain.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(d)
        .execute(&pool)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM redirects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_default_vhost_template_is_the_one_in_the_code_and_there_is_only_one(pool: PgPool) {
    let t = builtpanel_database::vhost_templates::default_for(&pool, "nginx")
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(t.body, builtpanel_domain::vhost::DEFAULT_NGINX_TEMPLATE);
    builtpanel_domain::vhost::check_template(&t.body).unwrap();
    // A second default for the same server is refused; a non-default is fine.
    let second = sqlx::query(
        "INSERT INTO vhost_templates (name, body, is_default) VALUES ('Other', 'x', true)",
    )
    .execute(&pool)
    .await;
    assert!(second.is_err());
    sqlx::query("INSERT INTO vhost_templates (name, body) VALUES ('Other', 'x')")
        .execute(&pool)
        .await
        .unwrap();
    // Names are plain and unique per server.
    for bad in ["", " x", "a/b", "a\nb"] {
        assert!(
            sqlx::query("INSERT INTO vhost_templates (name, body) VALUES ($1, 'x')")
                .bind(bad)
                .execute(&pool)
                .await
                .is_err(),
            "{bad:?}"
        );
    }
    assert!(
        sqlx::query("INSERT INTO vhost_templates (name, body) VALUES ('Other', 'y')")
            .execute(&pool)
            .await
            .is_err()
    );
}
