//! The ownership and duplication rules for domains (T05.02).
//! Ignored by default; see tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::domains::{self, AddError, NewDomain};
use builtpanel_domain::{Docroot, DomainKind, Fqdn};
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

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
        "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
    )
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
    name: &str,
    kind: DomainKind,
    parent: Option<Uuid>,
    sharing: bool,
) -> Result<domains::Domain, AddError> {
    let fqdn = Fqdn::parse(name).unwrap();
    let docroot = Docroot::default();
    let mut tx = pool.begin().await.unwrap();
    let d = domains::add(
        &mut tx,
        &NewDomain {
            tenant_id: tenant,
            fqdn: &fqdn,
            kind,
            parent_domain_id: parent,
            docroot: &docroot,
        },
        sharing,
    )
    .await?;
    tx.commit().await.unwrap();
    Ok(d)
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_name_exists_once_on_the_whole_panel(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let b = tenant(&pool, "bobby", 10_001).await;
    add(&pool, a, "example.com", DomainKind::Primary, None, false)
        .await
        .unwrap();
    for (who, kind) in [
        (a, DomainKind::Addon),
        (b, DomainKind::Primary),
        (b, DomainKind::Addon),
    ] {
        let err = add(&pool, who, "example.com", kind, None, false)
            .await
            .unwrap_err();
        assert!(matches!(err, AddError::Taken), "{err:?}");
    }
    // Even with sharing on, a name is still one tenant's.
    assert!(matches!(
        add(&pool, b, "example.com", DomainKind::Addon, None, true)
            .await
            .unwrap_err(),
        AddError::Taken
    ));
    // One primary each.
    assert!(matches!(
        add(&pool, a, "second.com", DomainKind::Primary, None, false)
            .await
            .unwrap_err(),
        AddError::PrimaryExists
    ));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_subdomain_of_someone_elses_domain_is_refused_unless_sharing_is_on(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let b = tenant(&pool, "bobby", 10_001).await;
    let main = add(&pool, a, "example.com", DomainKind::Primary, None, false)
        .await
        .unwrap();

    // As a subdomain of her own domain: fine.
    let blog = add(
        &pool,
        a,
        "blog.example.com",
        DomainKind::Sub,
        Some(main.id),
        false,
    )
    .await
    .unwrap();
    // Bobby cannot take anything below Alice's names, whatever kind he calls it
    // and however deep, and whichever of her names it hangs under.
    for (name, kind) in [
        ("shop.example.com", DomainKind::Addon),
        ("shop.example.com", DomainKind::Primary),
        ("a.b.example.com", DomainKind::Addon),
        ("x.blog.example.com", DomainKind::Addon),
    ] {
        let err = add(&pool, b, name, kind, None, false).await.unwrap_err();
        assert!(matches!(err, AddError::OwnedByOther), "{name}: {err:?}");
    }
    // Naming her domain as parent looks the same as naming nothing.
    for parent in [main.id, blog.id, Uuid::now_v7()] {
        let err = add(
            &pool,
            b,
            "www.example.com",
            DomainKind::Sub,
            Some(parent),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AddError::ParentNotFound), "{err:?}");
        let err = add(
            &pool,
            b,
            "elsewhere.net",
            DomainKind::Alias,
            Some(parent),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AddError::ParentNotFound), "{err:?}");
    }
    // Names that only end with the same letters are not below her domain.
    assert!(
        add(&pool, b, "myexample.com", DomainKind::Primary, None, false)
            .await
            .is_ok()
    );
    // With sharing permitted Bobby may host a subdomain of her domain.
    let shared = add(&pool, b, "shop.example.com", DomainKind::Addon, None, true)
        .await
        .unwrap();
    assert_eq!(shared.tenant_id, b);
    // The parent may still be added by its rightful owner later.
    let c = tenant(&pool, "carol", 10_002).await;
    assert!(
        add(&pool, c, "net.org", DomainKind::Primary, None, false)
            .await
            .is_ok()
    );
    assert!(
        add(
            &pool,
            c,
            "www.net.org",
            DomainKind::Sub,
            Some(shared.id),
            false
        )
        .await
        .is_err()
    );
    // A tenant that has the parent may add a name above its child owned by another.
    let d = tenant(&pool, "dave", 10_003).await;
    add(
        &pool,
        d,
        "deep.child.shared.org",
        DomainKind::Primary,
        None,
        false,
    )
    .await
    .unwrap();
    assert!(
        add(&pool, d, "shared.org", DomainKind::Addon, None, false)
            .await
            .is_ok()
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn parents_must_fit_the_kind_and_the_name(pool: PgPool) {
    let a = tenant(&pool, "alice", 10_000).await;
    let main = add(&pool, a, "example.com", DomainKind::Primary, None, false)
        .await
        .unwrap();
    // A subdomain's name must lie below its parent.
    assert!(matches!(
        add(&pool, a, "other.org", DomainKind::Sub, Some(main.id), false)
            .await
            .unwrap_err(),
        AddError::NotBelowParent
    ));
    assert!(matches!(
        add(
            &pool,
            a,
            "example.com.evil.org",
            DomainKind::Sub,
            Some(main.id),
            false
        )
        .await
        .unwrap_err(),
        AddError::NotBelowParent
    ));
    // Parent or none, by kind.
    assert!(matches!(
        add(&pool, a, "blog.example.com", DomainKind::Sub, None, false)
            .await
            .unwrap_err(),
        AddError::ParentMismatch
    ));
    assert!(matches!(
        add(
            &pool,
            a,
            "second.org",
            DomainKind::Addon,
            Some(main.id),
            false
        )
        .await
        .unwrap_err(),
        AddError::ParentMismatch
    ));
    // An alias is a name for a site; nothing hangs off an alias.
    let alias = add(
        &pool,
        a,
        "alias.net",
        DomainKind::Alias,
        Some(main.id),
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        add(
            &pool,
            a,
            "x.alias.net",
            DomainKind::Sub,
            Some(alias.id),
            false
        )
        .await
        .unwrap_err(),
        AddError::ParentNotFound
    ));
    assert_eq!(
        domains::find(&pool, &Fqdn::parse("alias.net").unwrap())
            .await
            .unwrap()
            .unwrap()
            .id,
        alias.id
    );
    assert!(domains::get(&pool, Uuid::now_v7()).await.unwrap().is_none());
}
