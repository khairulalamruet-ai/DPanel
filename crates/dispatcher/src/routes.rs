//! Working out a tenant's web routes from the panel's tables (T05.14b).

use std::collections::BTreeMap;

use builtpanel_database::routes::{RoutedDomain, RoutedRedirect};
use builtpanel_node_protocol::{
    MAX_HOSTS_PER_TENANT, MAX_REDIRECTS_PER_TENANT, MAX_SITES_PER_TENANT, RouteRedirect, SiteRoute,
};
use uuid::Uuid;

/// Puts the firewall policy into the sites: `by_host` is what applies to each domain
/// (the primary name of a site is the key; a mode of `off` is simply not in it).
pub fn apply_waf(
    routes: &mut [SiteRoute],
    by_host: &BTreeMap<String, builtpanel_node_protocol::WafSetting>,
) {
    for site in routes {
        site.waf = site.hosts.first().and_then(|h| by_host.get(h)).cloned();
    }
}

/// Puts the turned-away addresses into the sites: `by_host` is the list of each domain (the
/// primary name of a site is the key).
pub fn apply_ip_blocks(routes: &mut [SiteRoute], by_host: &BTreeMap<String, Vec<String>>) {
    for site in routes {
        if let Some(list) = site.hosts.first().and_then(|h| by_host.get(h)) {
            site.blocked_ips = list
                .iter()
                .take(builtpanel_node_protocol::MAX_BLOCKED_IPS)
                .cloned()
                .collect();
        }
    }
}

/// The sites a tenant has on its node.
///
/// * only an `active` or `suspended` tenant has any (its containers exist);
/// * every domain that is not an alias is a site; its aliases answer under the
///   same site, with the same redirects;
/// * a site of a suspended tenant, or of a suspended domain (or whose parent is
///   suspended, for an alias), shows the "suspended" page instead of being served;
/// * what does not fit the limits the agent enforces is left out, from the end
///   of the list, rather than failing the whole tenant.
#[must_use]
pub fn build(
    tenant_status: &str,
    upstream: &str,
    domains: &[RoutedDomain],
    redirects: &[RoutedRedirect],
) -> Vec<SiteRoute> {
    if !matches!(tenant_status, "active" | "suspended") {
        return Vec::new();
    }
    let tenant_suspended = tenant_status == "suspended";
    let mut suspended: BTreeMap<Uuid, bool> = BTreeMap::new();
    for d in domains.iter().filter(|d| d.kind != "alias") {
        suspended.insert(d.id, tenant_suspended || d.status == "suspended");
    }
    let mut sites: Vec<(Uuid, SiteRoute)> = Vec::new();
    for d in domains.iter().filter(|d| d.kind != "alias") {
        let route = SiteRoute {
            waf: None,
            blocked_ips: Vec::new(),
            hosts: vec![d.fqdn.clone()],
            upstream: (!suspended[&d.id]).then(|| upstream.to_owned()),
            redirects: redirects
                .iter()
                .filter(|r| r.domain_id == d.id)
                .filter_map(|r| {
                    Some(RouteRedirect {
                        path: r.path.clone(),
                        target: r.target.clone(),
                        code: u16::try_from(r.code).ok()?,
                    })
                })
                .collect(),
            force_https: d.force_https,
            hsts: d.hsts,
            // Who suspended it decides which page visitors see.
            suspended_by: suspended[&d.id].then(|| {
                if tenant_suspended {
                    "account"
                } else {
                    "domain"
                }
                .to_owned()
            }),
        };
        sites.push((d.id, route));
    }
    for alias in domains.iter().filter(|d| d.kind == "alias") {
        let Some(parent) = alias.parent_id else {
            continue;
        };
        let alias_suspended = alias.status == "suspended";
        if let Some((_, site)) = sites.iter_mut().find(|(id, _)| *id == parent) {
            // An alias that is itself suspended stops answering; one of a suspended
            // site shows the same page as the site.
            if !alias_suspended {
                site.hosts.push(alias.fqdn.clone());
            }
        }
    }
    let mut out: Vec<SiteRoute> = sites.into_iter().map(|(_, site)| site).collect();
    out.sort_by(|a, b| a.hosts[0].cmp(&b.hosts[0]));
    let (mut hosts, mut redirect_count) = (0usize, 0usize);
    out.truncate(MAX_SITES_PER_TENANT);
    out.retain_mut(|site| {
        if hosts + site.hosts.len() > MAX_HOSTS_PER_TENANT {
            return false;
        }
        hosts += site.hosts.len();
        let room = MAX_REDIRECTS_PER_TENANT.saturating_sub(redirect_count);
        site.redirects.truncate(room);
        redirect_count += site.redirects.len();
        true
    });
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dom(n: u128, fqdn: &str, kind: &str, parent: Option<u128>, status: &str) -> RoutedDomain {
        RoutedDomain {
            id: Uuid::from_u128(n),
            fqdn: fqdn.into(),
            kind: kind.into(),
            parent_id: parent.map(Uuid::from_u128),
            status: status.into(),
            force_https: false,
            hsts: false,
            docroot: String::new(),
            php_version: None,
            app_upstream: None,
        }
    }

    fn redirect(domain: u128, path: &str, target: &str) -> RoutedRedirect {
        RoutedRedirect {
            domain_id: Uuid::from_u128(domain),
            path: path.into(),
            target: target.into(),
            code: 301,
        }
    }

    const UP: &str = "bp-alice-web:8080";

    #[test]
    fn domains_become_sites_aliases_join_their_domain() {
        let domains = vec![
            dom(1, "example.com", "primary", None, "active"),
            dom(2, "blog.example.com", "sub", Some(1), "active"),
            dom(3, "example.net", "alias", Some(1), "active"),
            dom(4, "other.org", "addon", None, "active"),
        ];
        let routes = build("active", UP, &domains, &[redirect(1, "/old", "/new")]);
        let hosts: Vec<Vec<&str>> = routes
            .iter()
            .map(|r| r.hosts.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            hosts,
            [
                vec!["blog.example.com"],
                vec!["example.com", "example.net"],
                vec!["other.org"]
            ]
        );
        assert!(routes.iter().all(|r| r.upstream.as_deref() == Some(UP)));
        let main = &routes[1];
        assert_eq!(main.redirects.len(), 1);
        assert_eq!(
            (main.redirects[0].path.as_str(), main.redirects[0].code),
            ("/old", 301)
        );
        assert!(routes[0].redirects.is_empty());
    }

    #[test]
    fn the_https_settings_of_a_domain_go_to_its_site_and_its_aliases_share_them() {
        let mut main = dom(1, "example.com", "primary", None, "active");
        main.force_https = true;
        main.hsts = true;
        let mut forced = dom(3, "forced.org", "addon", None, "active");
        forced.force_https = true;
        let domains = vec![
            main,
            dom(2, "example.net", "alias", Some(1), "active"),
            forced,
            dom(4, "plain.org", "addon", None, "active"),
        ];
        let routes = build("active", UP, &domains, &[]);
        let by_host = |h: &str| routes.iter().find(|r| r.hosts[0] == h).unwrap();
        assert!(by_host("example.com").force_https && by_host("example.com").hsts);
        assert_eq!(by_host("example.com").hosts, ["example.com", "example.net"]);
        assert!(by_host("forced.org").force_https && !by_host("forced.org").hsts);
        assert!(!by_host("plain.org").force_https && !by_host("plain.org").hsts);
    }

    #[test]
    fn suspension_shows_the_page_for_the_tenant_or_one_domain() {
        let domains = vec![
            dom(1, "a.com", "primary", None, "active"),
            dom(2, "b.com", "addon", None, "suspended"),
            dom(3, "a.net", "alias", Some(2), "active"),
        ];
        let routes = build("active", UP, &domains, &[]);
        assert_eq!(routes[0].upstream.as_deref(), Some(UP));
        assert_eq!(
            routes[1].hosts,
            ["b.com", "a.net"],
            "an alias follows its domain"
        );
        assert_eq!(routes[1].upstream, None);
        let all = build("suspended", UP, &domains, &[]);
        assert!(
            all.iter().all(|r| r.upstream.is_none()),
            "a suspended tenant: every site"
        );
        // An alias that is itself suspended stops answering.
        let stopped = vec![
            dom(1, "a.com", "primary", None, "active"),
            dom(2, "a.net", "alias", Some(1), "suspended"),
        ];
        assert_eq!(build("active", UP, &stopped, &[])[0].hosts, ["a.com"]);
    }

    #[test]
    fn a_tenant_that_is_not_running_has_no_routes() {
        let domains = vec![dom(1, "a.com", "primary", None, "active")];
        for status in ["provisioning", "deleting", "failed", "nonsense"] {
            assert!(
                build(status, UP, &domains, &[redirect(1, "/a", "/b")]).is_empty(),
                "{status}"
            );
        }
        assert!(build("active", UP, &[], &[]).is_empty());
    }

    #[test]
    fn what_does_not_fit_the_agents_limits_is_left_out_not_fatal() {
        let many: Vec<RoutedDomain> = (0..300u128)
            .map(|i| dom(i + 1, &format!("site{i:03}.com"), "addon", None, "active"))
            .collect();
        let routes = build("active", UP, &many, &[]);
        assert_eq!(routes.len(), MAX_SITES_PER_TENANT);
        assert_eq!(
            routes[0].hosts[0], "site000.com",
            "from the end of the list"
        );
        let manyr: Vec<RoutedRedirect> = (0..600)
            .map(|i| redirect(1, &format!("/p{i:03}"), "/x"))
            .collect();
        let routes = build(
            "active",
            UP,
            &[dom(1, "a.com", "primary", None, "active")],
            &manyr,
        );
        assert_eq!(routes[0].redirects.len(), MAX_REDIRECTS_PER_TENANT);
    }

    #[test]
    fn who_suspended_a_site_decides_its_page() {
        let domains = vec![
            dom(1, "paused.org", "primary", None, "suspended"),
            dom(2, "fine.org", "addon", None, "active"),
        ];
        let by_domain = build("active", UP, &domains, &[]);
        let paused = by_domain
            .iter()
            .find(|s| s.hosts[0] == "paused.org")
            .unwrap();
        let fine = by_domain.iter().find(|s| s.hosts[0] == "fine.org").unwrap();
        assert_eq!(paused.suspended_by.as_deref(), Some("domain"));
        assert_eq!(fine.suspended_by, None);
        // With the whole account suspended every site says so.
        let by_account = build("suspended", UP, &domains, &[]);
        assert!(
            by_account
                .iter()
                .all(|s| s.suspended_by.as_deref() == Some("account"))
        );
    }

    #[test]
    fn turned_away_addresses_go_to_the_site_of_their_primary_name_only() {
        let domains = [
            dom(1, "a.com", "primary", None, "active"),
            dom(2, "b.com", "primary", None, "active"),
        ];
        let mut routes = build("active", UP, &domains, &[]);
        let mut by_host = BTreeMap::new();
        by_host.insert(
            "a.com".to_owned(),
            vec!["203.0.113.9".to_owned(), "198.51.100.0/24".to_owned()],
        );
        apply_ip_blocks(&mut routes, &by_host);
        assert_eq!(routes[0].blocked_ips, ["203.0.113.9", "198.51.100.0/24"]);
        assert!(routes[1].blocked_ips.is_empty());
    }

    #[test]
    fn the_firewall_setting_goes_to_the_site_of_its_primary_name_only() {
        use builtpanel_node_protocol::WafSetting;
        let domains = [
            dom(1, "a.com", "primary", None, "active"),
            dom(2, "b.com", "primary", None, "active"),
        ];
        let mut routes = build("active", UP, &domains, &[]);
        let mut by_host = BTreeMap::new();
        by_host.insert(
            "a.com".to_owned(),
            WafSetting {
                mode: "block".into(),
                disabled_rules: vec![942100],
            },
        );
        apply_waf(&mut routes, &by_host);
        assert_eq!(
            routes[0].waf.as_ref().map(|w| w.mode.as_str()),
            Some("block")
        );
        assert_eq!(
            routes[0].waf.as_ref().map(|w| w.disabled_rules.clone()),
            Some(vec![942100])
        );
        assert!(routes[1].waf.is_none());
    }
}
