//! The `domains` table and the rules for adding to it (T05.02).
//!
//! One name is served by one tenant on the whole panel, and a tenant may only
//! put a name below one of its own domains. The rules live here, in one function
//! ([`add`]), so the client API, the admin API, bulk import and the command line
//! cannot disagree about them.
//!
//! * A name that exists anywhere is taken ([`AddError::Taken`]).
//! * `sub` and `alias` belong to a domain of the same tenant; a `sub` lies below
//!   it. Naming somebody else's domain as parent is the same answer as naming a
//!   domain that does not exist, so nobody can probe what others host.
//! * A name below a domain that **another** tenant has ([`AddError::OwnedByOther`],
//!   e.g. `shop.example.com` while another tenant has `example.com`) is refused,
//!   unless the panel permits subdomain sharing (setting
//!   `permit_subdomain_sharing`, off by default).
//! * One `primary` per tenant.

use builtpanel_domain::{Docroot, DomainKind, DomainStatus, Fqdn};
use sqlx::{PgConnection, PgPool, types::Uuid};

/// A domain row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    /// Id.
    pub id: Uuid,
    /// The tenant it belongs to.
    pub tenant_id: Uuid,
    /// The name, lower-case ASCII.
    pub fqdn: String,
    /// How it relates to its tenant.
    pub kind: DomainKind,
    /// The domain a `sub` or `alias` belongs to.
    pub parent_domain_id: Option<Uuid>,
    /// Files, relative to the tenant's web directory.
    pub docroot: String,
    /// Whether it is served.
    pub status: DomainStatus,
    /// Visitors on plain HTTP are sent to HTTPS.
    pub force_https: bool,
    /// Answers over HTTPS carry the HSTS header (needs `force_https`).
    pub hsts: bool,
    /// Creation time (Unix seconds).
    pub created_at: i64,
}

/// What to add.
#[derive(Debug, Clone)]
pub struct NewDomain<'a> {
    /// The tenant.
    pub tenant_id: Uuid,
    /// The (validated) name.
    pub fqdn: &'a Fqdn,
    /// The kind.
    pub kind: DomainKind,
    /// The domain a `sub` or `alias` belongs to.
    pub parent_domain_id: Option<Uuid>,
    /// Files, relative to the tenant's web directory.
    pub docroot: &'a Docroot,
}

/// Why [`add`] refused.
#[derive(Debug, thiserror::Error)]
pub enum AddError {
    /// The name exists already (on this or another tenant).
    #[error("this domain is already in use")]
    Taken,
    /// The name lies below a domain of another tenant.
    #[error("this domain lies below a domain that belongs to someone else")]
    OwnedByOther,
    /// `sub` and `alias` need a parent, the other kinds must not have one.
    #[error(
        "a subdomain or alias needs the domain it belongs to, and other domains must not name one"
    )]
    ParentMismatch,
    /// The parent does not exist, is not this tenant's, or is an alias.
    #[error("the parent domain was not found")]
    ParentNotFound,
    /// A `sub` whose name is not below its parent's.
    #[error("a subdomain must lie below its parent domain")]
    NotBelowParent,
    /// The tenant has a primary domain already.
    #[error("this account already has a primary domain")]
    PrimaryExists,
    /// The tenant does not exist.
    #[error("unknown tenant")]
    UnknownTenant,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

struct Row {
    id: Uuid,
    tenant_id: Uuid,
    fqdn: String,
    kind: String,
    parent_domain_id: Option<Uuid>,
    docroot: String,
    status: String,
    force_https: bool,
    hsts: bool,
    created_at: i64,
}

impl From<Row> for Domain {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            fqdn: r.fqdn,
            kind: DomainKind::parse(&r.kind).unwrap_or(DomainKind::Addon),
            parent_domain_id: r.parent_domain_id,
            docroot: r.docroot,
            status: DomainStatus::parse(&r.status).unwrap_or(DomainStatus::Active),
            force_https: r.force_https,
            hsts: r.hsts,
            created_at: r.created_at,
        }
    }
}

/// Advisory lock namespace for adding domains below one registrable name.
const NAME_LOCK: i32 = 7_305_001;

/// Adds a domain under the rules in the module documentation. Call it in a
/// transaction: the work of two requests below the same registrable name is
/// serialised until it ends.
///
/// # Errors
/// [`AddError`].
pub async fn add(
    conn: &mut PgConnection,
    new: &NewDomain<'_>,
    permit_subdomain_sharing: bool,
) -> Result<Domain, AddError> {
    if new.kind.needs_parent() != new.parent_domain_id.is_some() {
        return Err(AddError::ParentMismatch);
    }
    // Two requests for names below the same registrable name wait for each other,
    // so the ownership check below cannot be raced.
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        NAME_LOCK,
        new.fqdn.registrable(),
    )
    .execute(&mut *conn)
    .await?;

    if let Some(parent_id) = new.parent_domain_id {
        let parent = sqlx::query!(
            "SELECT fqdn, kind FROM domains WHERE id = $1 AND tenant_id = $2",
            parent_id,
            new.tenant_id,
        )
        .fetch_optional(&mut *conn)
        .await?
        .filter(|p| p.kind != "alias")
        .ok_or(AddError::ParentNotFound)?;
        if new.kind == DomainKind::Sub {
            let below = Fqdn::parse(&parent.fqdn)
                .map(|p| new.fqdn.is_below(&p))
                .unwrap_or(false);
            if !below {
                return Err(AddError::NotBelowParent);
            }
        }
    }

    if !permit_subdomain_sharing {
        // Is some ancestor of the new name (`a.b.example.com` -> `b.example.com`,
        // `example.com`) a domain of another tenant?
        let owned = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM domains
                   WHERE tenant_id <> $1 AND $2 LIKE '%.' || fqdn
               ) AS "owned!""#,
            new.tenant_id,
            new.fqdn.as_str(),
        )
        .fetch_one(&mut *conn)
        .await?;
        if owned {
            return Err(AddError::OwnedByOther);
        }
    }

    let row = sqlx::query_as!(
        Row,
        r#"INSERT INTO domains (tenant_id, fqdn, kind, parent_domain_id, docroot)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, tenant_id, fqdn, kind, parent_domain_id, docroot, status, force_https, hsts,
                     extract(epoch FROM created_at)::bigint AS "created_at!""#,
        new.tenant_id,
        new.fqdn.as_str(),
        new.kind.as_str(),
        new.parent_domain_id,
        new.docroot.as_str(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(
        |err| match err.as_database_error().map(|e| (e.constraint(), e.code())) {
            Some((Some("domains_fqdn_key"), _)) => AddError::Taken,
            Some((Some("domains_one_primary_per_tenant"), _)) => AddError::PrimaryExists,
            Some((Some("domains_tenant_id_fkey"), _)) => AddError::UnknownTenant,
            _ => AddError::Database(err),
        },
    )?;
    Ok(row.into())
}

/// One domain by id.
///
/// # Errors
/// Database errors.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Domain>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT id, tenant_id, fqdn, kind, parent_domain_id, docroot, status, force_https, hsts,
                  extract(epoch FROM created_at)::bigint AS "created_at!"
           FROM domains WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Domain::from))
}

/// The domain with this name, if any.
///
/// # Errors
/// Database errors.
pub async fn find(pool: &PgPool, fqdn: &Fqdn) -> Result<Option<Domain>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT id, tenant_id, fqdn, kind, parent_domain_id, docroot, status, force_https, hsts,
                  extract(epoch FROM created_at)::bigint AS "created_at!"
           FROM domains WHERE fqdn = $1"#,
        fqdn.as_str(),
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Domain::from))
}

/// A domain with the account it belongs to, for lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainListing {
    /// The domain.
    pub domain: Domain,
    /// System name of its tenant.
    pub tenant_username: String,
    /// When it was suspended (Unix seconds).
    pub suspended_at: Option<i64>,
}

/// What to look for in [`list`].
#[derive(Debug, Clone, Default)]
pub struct Filter<'a> {
    /// Only this tenant's domains.
    pub tenant_id: Option<Uuid>,
    /// Only this kind.
    pub kind: Option<DomainKind>,
    /// Only this status.
    pub status: Option<DomainStatus>,
    /// Text to find in the name.
    pub search: Option<&'a str>,
}

fn like_pattern(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Domains by name, and how many match in all.
///
/// # Errors
/// Database errors.
pub async fn list(
    pool: &PgPool,
    filter: &Filter<'_>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<DomainListing>, i64), sqlx::Error> {
    let kind = filter.kind.map(DomainKind::as_str);
    let status = filter.status.map(DomainStatus::as_str);
    let pattern = filter.search.map(like_pattern);
    let total = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM domains d
           WHERE ($1::uuid IS NULL OR d.tenant_id = $1)
             AND ($2::text IS NULL OR d.kind = $2)
             AND ($3::text IS NULL OR d.status = $3)
             AND ($4::text IS NULL OR d.fqdn ILIKE $4)"#,
        filter.tenant_id,
        kind,
        status,
        pattern,
    )
    .fetch_one(pool)
    .await?;
    let rows = sqlx::query!(
        r#"SELECT d.id, d.tenant_id, d.fqdn, d.kind, d.parent_domain_id, d.docroot, d.status,
                  d.force_https, d.hsts,
                  extract(epoch FROM d.created_at)::bigint AS "created_at!",
                  extract(epoch FROM d.suspended_at)::bigint AS suspended_at,
                  t.username AS tenant_username
           FROM domains d JOIN tenants t ON t.id = d.tenant_id
           WHERE ($1::uuid IS NULL OR d.tenant_id = $1)
             AND ($2::text IS NULL OR d.kind = $2)
             AND ($3::text IS NULL OR d.status = $3)
             AND ($4::text IS NULL OR d.fqdn ILIKE $4)
           ORDER BY d.fqdn, d.id
           LIMIT $5 OFFSET $6"#,
        filter.tenant_id,
        kind,
        status,
        pattern,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|r| DomainListing {
            domain: Domain::from(Row {
                id: r.id,
                tenant_id: r.tenant_id,
                fqdn: r.fqdn,
                kind: r.kind,
                parent_domain_id: r.parent_domain_id,
                docroot: r.docroot,
                status: r.status,
                force_https: r.force_https,
                hsts: r.hsts,
                created_at: r.created_at,
            }),
            tenant_username: r.tenant_username,
            suspended_at: r.suspended_at,
        })
        .collect();
    Ok((items, total))
}

/// How many domains a tenant has of each kind: `(primary, addon, sub, alias)`.
///
/// # Errors
/// Database errors.
pub async fn counts(
    conn: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<(i64, i64, i64, i64), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT count(*) FILTER (WHERE kind = 'primary') AS "primary!",
                  count(*) FILTER (WHERE kind = 'addon') AS "addon!",
                  count(*) FILTER (WHERE kind = 'sub') AS "sub!",
                  count(*) FILTER (WHERE kind = 'alias') AS "alias!"
           FROM domains WHERE tenant_id = $1"#,
        tenant_id,
    )
    .fetch_one(conn)
    .await?;
    Ok((row.primary, row.addon, row.sub, row.alias))
}

/// Serialises the changes of one tenant's domains (limits are checked against a
/// count that must not change under the check). Call it first in a transaction.
///
/// # Errors
/// Database errors.
pub async fn lock_tenant(conn: &mut PgConnection, tenant_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2::text))",
        TENANT_LOCK,
        tenant_id.to_string(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Advisory lock namespace for one tenant's domain count.
const TENANT_LOCK: i32 = 7_305_002;

/// Removes a domain and what hangs off it (subdomains, aliases, redirects).
/// Returns whether it existed.
///
/// # Errors
/// Database errors.
pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!("DELETE FROM domains WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Suspends or resumes a domain. Returns whether it exists.
///
/// # Errors
/// Database errors.
pub async fn set_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: DomainStatus,
) -> Result<bool, sqlx::Error> {
    let suspended = status == DomainStatus::Suspended;
    let done = sqlx::query!(
        "UPDATE domains
         SET status = $2, suspended_at = CASE WHEN $3 THEN COALESCE(suspended_at, now()) ELSE NULL END
         WHERE id = $1",
        id,
        status.as_str(),
        suspended,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Changes where a domain's files are. Returns whether it exists.
///
/// # Errors
/// Database errors.
pub async fn set_docroot(
    conn: &mut PgConnection,
    id: Uuid,
    docroot: &Docroot,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE domains SET docroot = $2 WHERE id = $1",
        id,
        docroot.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Whether a certificate may be issued for `fqdn`: it is a domain of a hosting
/// account that is running or suspended (a suspended site still shows its page
/// over HTTPS), and the domain itself is active.
///
/// # Errors
/// Database errors.
pub async fn certificate_allowed(pool: &PgPool, fqdn: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM domains d JOIN tenants t ON t.id = d.tenant_id
               WHERE d.fqdn = $1 AND d.status = 'active' AND t.status IN ('active', 'suspended')
           ) OR EXISTS (
               SELECT 1 FROM preview_links p JOIN tenants t ON t.id = p.tenant_id
               WHERE p.host = $1 AND p.expires_at > now() AND t.status IN ('active', 'suspended')
           ) AS "allowed!""#,
        fqdn,
    )
    .fetch_one(pool)
    .await
}

/// Changes a domain's HTTPS settings (HSTS needs forced HTTPS: the table refuses
/// the other combination). Returns whether it exists.
///
/// # Errors
/// Database errors.
pub async fn set_https(
    conn: &mut PgConnection,
    id: Uuid,
    force_https: bool,
    hsts: bool,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE domains SET force_https = $2, hsts = $3 WHERE id = $1",
        id,
        force_https,
        hsts,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}
