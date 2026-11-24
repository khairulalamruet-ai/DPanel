//! A tenant's web server configuration, made from the panel's tables (T06.04b).
//!
//! Used by the dispatcher (to send the files to the node) and by the API (to show
//! a person the file their site runs on).

use builtpanel_database::{domain_policies::DomainPolicy, routes::RoutedDomain};
use builtpanel_domain::{Docroot, Fqdn, TenantUsername, vhost};
use builtpanel_node_protocol::MAX_HOSTS_PER_TENANT;
use uuid::Uuid;

/// The configuration of every site of a tenant: `(domain, file content)`, in the
/// order of the domains' names.
///
/// * only an `active` or `suspended` tenant has any (the containers exist);
/// * every domain that is not an alias is a site, answering also for its aliases;
/// * a domain whose docroot or name does not pass the checks is left out (the
///   others still get theirs), and so is what does not fit the node's limits.
#[must_use]
#[allow(clippy::too_many_arguments)] // each is one table of the tenant
pub fn build(
    tenant_status: &str,
    template: &str,
    username: &TenantUsername,
    account_php: &str,
    domains: &[RoutedDomain],
    policies: &[(Uuid, DomainPolicy)],
    customs: &[(Uuid, String)],
    caches: &[(Uuid, builtpanel_database::page_cache::PageCache)],
) -> Vec<(String, String)> {
    if !matches!(tenant_status, "active" | "suspended") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for d in domains.iter().filter(|d| d.kind != "alias") {
        let Ok(domain) = Fqdn::parse(&d.fqdn) else {
            continue;
        };
        let Ok(docroot) = Docroot::parse(&d.docroot) else {
            continue;
        };
        let aliases: Vec<Fqdn> = domains
            .iter()
            .filter(|a| a.kind == "alias" && a.parent_id == Some(d.id) && a.status != "suspended")
            .filter_map(|a| Fqdn::parse(&a.fqdn).ok())
            .collect();
        // A policy that does not pass the checks (it was checked when it was
        // saved) is left out rather than losing the whole site.
        let policy = policies
            .iter()
            .find(|(id, _)| *id == d.id)
            .and_then(|(_, p)| {
                vhost::Policy::parse(
                    p.rate_limit_per_second,
                    p.headers
                        .iter()
                        .map(|h| vhost::Header {
                            name: h.name.clone(),
                            value: h.value.clone(),
                        })
                        .collect(),
                )
                .ok()
            })
            .unwrap_or_default()
            .with_page_cache(
                caches
                    .iter()
                    .find(|(id, _)| *id == d.id)
                    .and_then(|(_, c)| {
                        vhost::PageCache::parse(
                            u16::try_from(c.ttl_minutes).ok()?,
                            u32::try_from(c.generation).ok()?,
                        )
                        .ok()
                    }),
            );
        // The person's own block was checked when it was saved; one that no longer
        // passes (the rules got stricter) is left out, the site stays.
        let custom = customs
            .iter()
            .find(|(id, _)| *id == d.id)
            .map(|(_, text)| text.as_str())
            .filter(|text| vhost::check_custom_block(text).is_ok())
            .unwrap_or("");
        // The PHP container of the site's version: the account's own, or the one
        // its domain asked for.
        let Ok(php_upstream) = builtpanel_tenant_stack::php_upstream_for(
            username,
            d.php_version.as_deref(),
            account_php,
        ) else {
            continue;
        };
        let site = vhost::Site {
            domain: &domain,
            aliases: &aliases,
            docroot: &docroot,
            php_upstream: &php_upstream,
            policy: &policy,
            custom,
            proxy: d.app_upstream.as_deref(),
        };
        if let Ok(config) = vhost::render(template, &site) {
            out.push((domain.as_str().to_owned(), config));
        }
    }
    out.sort();
    out.truncate(MAX_HOSTS_PER_TENANT);
    out
}

/// Reads what the files are made from and makes them for a tenant.
///
/// # Errors
/// A database error, as text.
pub async fn site_files(
    db: &builtpanel_database::sqlx::PgPool,
    tenant_id: Uuid,
    username: &str,
    status: &str,
) -> Result<Vec<(String, String)>, String> {
    let username = builtpanel_domain::TenantUsername::parse(username).map_err(|e| e.to_string())?;
    let template = builtpanel_database::vhost_templates::default_for(db, "nginx")
        .await
        .map_err(|e| e.to_string())?
        .map_or_else(|| vhost::DEFAULT_NGINX_TEMPLATE.to_owned(), |t| t.body);
    let account_php = builtpanel_database::php::state(db, tenant_id)
        .await
        .map_err(|e| e.to_string())?
        .map_or_else(|| "8.4".to_owned(), |s| s.version);
    let domains = builtpanel_database::routes::domains(db, tenant_id)
        .await
        .map_err(|e| e.to_string())?;
    let policies = builtpanel_database::domain_policies::of_tenant(db, tenant_id)
        .await
        .map_err(|e| e.to_string())?;
    let customs = builtpanel_database::domain_policies::custom_blocks_of_tenant(db, tenant_id)
        .await
        .map_err(|e| e.to_string())?;
    let caches = builtpanel_database::page_cache::of_tenant(db, tenant_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(build(
        status,
        &template,
        &username,
        &account_php,
        &domains,
        &policies,
        &customs,
        &caches,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use builtpanel_domain::vhost::DEFAULT_NGINX_TEMPLATE;
    use uuid::Uuid;

    use super::*;

    fn dom(n: u128, fqdn: &str, kind: &str, parent: Option<u128>, docroot: &str) -> RoutedDomain {
        RoutedDomain {
            id: Uuid::from_u128(n),
            fqdn: fqdn.into(),
            kind: kind.into(),
            parent_id: parent.map(Uuid::from_u128),
            status: "active".into(),
            force_https: false,
            hsts: false,
            docroot: docroot.into(),
            php_version: None,
            app_upstream: None,
        }
    }

    fn alice() -> TenantUsername {
        TenantUsername::parse("alice").unwrap()
    }

    #[test]
    fn a_domain_with_a_running_application_is_passed_on_to_it() {
        let mut app = dom(1, "app.example.com", "primary", None, "");
        app.app_upstream = Some("bp-alice-app-api:3000".into());
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &[app, dom(2, "www.example.com", "addon", None, "site")],
            &[],
            &[],
            &[],
        );
        assert_eq!(files.len(), 2);
        let by = |name: &str| files.iter().find(|(d, _)| d == name).unwrap().1.clone();
        assert!(by("app.example.com").contains("proxy_pass http://bp-alice-app-api:3000;"));
        assert!(!by("app.example.com").contains("fastcgi_pass"));
        // The other site is still served from its files.
        assert!(by("www.example.com").contains("fastcgi_pass"));
    }

    #[test]
    fn every_site_gets_a_file_and_aliases_share_their_domains() {
        let domains = vec![
            dom(1, "example.com", "primary", None, "public_html"),
            dom(2, "www.example.com", "alias", Some(1), ""),
            dom(3, "shop.org", "addon", None, "shop"),
            dom(4, "blog.example.com", "sub", Some(1), "blog"),
        ];
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &domains,
            &[],
            &[],
            &[],
        );
        let names: Vec<&str> = files.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(names, ["blog.example.com", "example.com", "shop.org"]);
        let main = &files[1].1;
        assert!(
            main.contains("server_name example.com www.example.com;"),
            "{main}"
        );
        assert!(main.contains("root /var/www/html/public_html;"));
        assert!(main.contains("fastcgi_pass bp-alice-php:9000;"));
        assert!(files[2].1.contains("root /var/www/html/shop;"));
    }

    #[test]
    fn a_tenant_without_containers_has_no_files_and_a_bad_domain_is_skipped() {
        let domains = vec![dom(1, "example.com", "primary", None, "")];
        for status in ["provisioning", "failed", "deleting"] {
            assert!(
                build(
                    status,
                    DEFAULT_NGINX_TEMPLATE,
                    &alice(),
                    "8.4",
                    &domains,
                    &[],
                    &[],
                    &[],
                )
                .is_empty()
            );
        }
        assert_eq!(
            build(
                "suspended",
                DEFAULT_NGINX_TEMPLATE,
                &alice(),
                "8.4",
                &domains,
                &[],
                &[],
                &[],
            )
            .len(),
            1
        );
        let bad = vec![
            dom(1, "example.com", "primary", None, "../etc"),
            dom(2, "ok.org", "primary", None, ""),
        ];
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &bad,
            &[],
            &[],
            &[],
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "ok.org");
    }

    #[test]
    fn a_suspended_alias_stops_answering() {
        let mut alias = dom(2, "www.example.com", "alias", Some(1), "");
        alias.status = "suspended".into();
        let domains = vec![dom(1, "example.com", "primary", None, ""), alias];
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &domains,
            &[],
            &[],
            &[],
        );
        assert!(files[0].1.contains("server_name example.com;"));
    }

    #[test]
    fn a_domains_policy_goes_into_its_file_only() {
        use builtpanel_database::domain_policies::StoredHeader;
        let domains = vec![
            dom(1, "example.com", "primary", None, ""),
            dom(2, "other.org", "addon", None, ""),
        ];
        let policy = DomainPolicy {
            rate_limit_per_second: Some(5),
            headers: vec![StoredHeader {
                name: "X-Test".into(),
                value: "yes".into(),
            }],
        };
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &domains,
            &[(Uuid::from_u128(1), policy)],
            &[],
            &[],
        );
        assert!(files[0].1.contains("add_header X-Test \"yes\" always;"));
        assert!(files[0].1.contains("rate=5r/s"));
        assert!(!files[1].1.contains("limit_req"));
        // A policy that cannot pass the checks is left out, the site stays.
        let broken = DomainPolicy {
            rate_limit_per_second: Some(0),
            headers: vec![],
        };
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &domains,
            &[(Uuid::from_u128(1), broken)],
            &[],
            &[],
        );
        assert_eq!(files.len(), 2);
        assert!(!files[0].1.contains("limit_req"));
    }

    #[test]
    fn a_domain_with_its_own_php_version_uses_that_versions_container() {
        let mut other = dom(2, "old.org", "addon", None, "");
        other.php_version = Some("8.3".into());
        let mut same = dom(3, "same.org", "addon", None, "");
        same.php_version = Some("8.4".into());
        let domains = vec![dom(1, "example.com", "primary", None, ""), other, same];
        let files = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.4",
            &domains,
            &[],
            &[],
            &[],
        );
        let of = |name: &str| &files.iter().find(|(d, _)| d == name).unwrap().1;
        assert!(of("example.com").contains("fastcgi_pass bp-alice-php:9000;"));
        assert!(of("old.org").contains("fastcgi_pass bp-alice-php-8-3:9000;"));
        assert!(
            of("same.org").contains("fastcgi_pass bp-alice-php:9000;"),
            "the account's own version is the main container"
        );
        // After the account moves to 8.3, the domain that asked for it follows the
        // main container and the others (now different) get their own.
        let moved = build(
            "active",
            DEFAULT_NGINX_TEMPLATE,
            &alice(),
            "8.3",
            &domains,
            &[],
            &[],
            &[],
        );
        let of = |name: &str| &moved.iter().find(|(d, _)| d == name).unwrap().1;
        assert!(of("old.org").contains("fastcgi_pass bp-alice-php:9000;"));
        assert!(of("example.com").contains("fastcgi_pass bp-alice-php:9000;"));
        assert!(of("same.org").contains("fastcgi_pass bp-alice-php-8-4:9000;"));
    }
}
