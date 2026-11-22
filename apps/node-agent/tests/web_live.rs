//! The web server operations against a real Caddy, with a real site behind it.
//! Ignored by default; needs a Caddy whose admin API and HTTP port the script
//! publishes (see deploy/dev/web-live.sh):
//!   BUILTPANEL_TEST_CADDY_ADMIN=http://127.0.0.1:12019
//!   BUILTPANEL_TEST_CADDY_URL=http://127.0.0.1:18080
//!   BUILTPANEL_TEST_CADDY_CONTAINER=<name of the caddy container>
//! and a container `bp-alice-web` (port 8080) on a network Caddy is also on.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{process::Command, time::Duration};

use builtpanel_node_agent::web::{
    Settings, WebContext, set_custom_certificates, set_error_pages, set_site_routes,
    set_user_agent_blocklist,
};
use builtpanel_node_protocol::{
    CustomCertificate, ErrorCode, ErrorPage, Operation, Output, RouteRedirect, SiteRoute,
};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// One HTTP request to Caddy with a chosen `Host`: `(status, location, body)`.
async fn get(url: &str, host: &str, path: &str) -> (u16, Option<String>, String) {
    let (url, host, path) = (url.to_owned(), host.to_owned(), path.to_owned());
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        let mut r = agent
            .get(&format!("{url}{path}"))
            .header("Host", &host)
            .call()
            .unwrap();
        let status = r.status().as_u16();
        let location = r
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        (
            status,
            location,
            r.body_mut().read_to_string().unwrap_or_default(),
        )
    })
    .await
    .unwrap()
}

fn site(hosts: &[&str], upstream: Option<&str>, redirects: Vec<RouteRedirect>) -> SiteRoute {
    SiteRoute {
        waf: None,
        blocked_ips: Vec::new(),
        hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        upstream: upstream.map(str::to_owned),
        redirects,
        force_https: false,
        hsts: false,
        suspended_by: None,
    }
}

fn set(username: &str, routes: Vec<SiteRoute>) -> Operation {
    Operation::SetSiteRoutes {
        username: username.into(),
        routes,
    }
}

#[tokio::test]
#[ignore = "needs a Caddy and a site behind it (see the file header)"]
async fn sites_are_served_suspended_redirected_and_put_back_after_caddy_forgets_them() {
    let (Some(admin), Some(url), Some(container)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_URL"),
        env("BUILTPANEL_TEST_CADDY_CONTAINER"),
    ) else {
        eprintln!("skipped: no Caddy (see the script)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-web-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin,
            listen: vec![":18080".into()],
            automatic_https: false,
            data_dir: None,
            waf_dir: None,
            panel: None,
        },
        &dir,
    );

    // Nothing yet: any name gets the plain "nothing here" page.
    web.reconcile().await.unwrap();
    let (status, _, body) = get(&url, "example.com", "/").await;
    assert_eq!(status, 404);
    assert!(body.contains("No site is set up"), "{body}");

    // A site: the tenant's web container answers for both names.
    let out = set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(
                &["example.com", "www.example.com"],
                Some("bp-alice-web:8080"),
                vec![
                    RouteRedirect {
                        path: "/old".into(),
                        target: "/new.html".into(),
                        code: 301,
                    },
                    RouteRedirect {
                        path: "/away".into(),
                        target: "https://example.org/there".into(),
                        code: 302,
                    },
                ],
            )],
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        Output::SiteRoutesSet {
            sites: 1,
            hosts: 2,
            applied: true
        }
    );
    for host in ["example.com", "www.example.com", "WWW.Example.COM"] {
        let (status, _, body) = get(&url, host, "/").await;
        assert_eq!((status, body.trim()), (200, "hello from alice"), "{host}");
    }
    // Redirects are answered before the site is asked, with the right code and Location.
    let (status, location, _) = get(&url, "example.com", "/old").await;
    assert_eq!((status, location.as_deref()), (301, Some("/new.html")));
    let (status, location, _) = get(&url, "example.com", "/away").await;
    assert_eq!(
        (status, location.as_deref()),
        (302, Some("https://example.org/there"))
    );
    // Only the exact path.
    assert_eq!(
        get(&url, "example.com", "/old/more").await.0,
        404,
        "the site has no such file"
    );
    // Other names still get nothing.
    assert_eq!(get(&url, "other.org", "/").await.0, 404);

    // Suspended: the page, not the files; and the right header.
    set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(&["example.com", "www.example.com"], None, vec![])],
        ),
    )
    .await
    .unwrap();
    let (status, _, body) = get(&url, "example.com", "/").await;
    assert_eq!(status, 503);
    assert!(body.contains("suspended"), "{body}");
    // Resumed.
    set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(
                &["example.com", "www.example.com"],
                Some("bp-alice-web:8080"),
                vec![],
            )],
        ),
    )
    .await
    .unwrap();
    assert_eq!(get(&url, "example.com", "/").await.0, 200);

    // Caddy forgets everything (restart without its saved configuration): the next
    // pass of the agent puts the sites back.
    let reset = Command::new("docker")
        .args([
            "exec",
            &container,
            "wget",
            "-q",
            "-O-",
            "--post-data",
            "",
            "--header=Content-Type: application/json",
            "--header=Origin: http://127.0.0.1:12019",
            "http://127.0.0.1:2019/stop",
        ])
        .output();
    let _ = reset;
    // Wait until a pass finds the routes gone and puts them back (`Ok(true)`): Caddy
    // is down for a moment while it restarts, then comes back with only its admin
    // settings.
    let mut put_back = false;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if web.reconcile().await == Ok(true) {
            put_back = true;
            break;
        }
    }
    assert!(
        put_back,
        "Caddy never lost the routes, or they were not put back"
    );
    assert_eq!(
        get(&url, "example.com", "/").await.0,
        200,
        "the site is back"
    );
    assert!(
        !web.reconcile().await.unwrap(),
        "nothing to do when everything is in place"
    );

    // A name belongs to one tenant; a route can only point at the tenant's own container.
    let err = set_site_routes(
        web.clone(),
        set(
            "bobby",
            vec![site(&["example.com"], Some("bp-bobby-web:8080"), vec![])],
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    let err = set_site_routes(
        web.clone(),
        set(
            "bobby",
            vec![site(&["bobs.net"], Some("bp-alice-web:8080"), vec![])],
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    assert_eq!(
        get(&url, "example.com", "/").await.0,
        200,
        "refused changes change nothing"
    );

    // Removing a tenant's routes takes the sites down.
    let out = set_site_routes(web.clone(), set("alice", vec![]))
        .await
        .unwrap();
    assert_eq!(
        out,
        Output::SiteRoutesSet {
            sites: 0,
            hosts: 0,
            applied: true
        }
    );
    assert_eq!(get(&url, "example.com", "/").await.0, 404);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "needs a Caddy (see the file header)"]
async fn routes_are_kept_when_caddy_cannot_be_reached_and_applied_later() {
    let dir = std::env::temp_dir().join(format!("bp-web-down-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin: "http://127.0.0.1:9".into(),
            listen: vec![":18080".into()],
            automatic_https: false,
            data_dir: None,
            waf_dir: None,
            panel: None,
        },
        &dir,
    );
    let out = set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(&["example.com"], Some("bp-alice-web:8080"), vec![])],
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        Output::SiteRoutesSet {
            sites: 1,
            hosts: 1,
            applied: false
        },
        "kept, not applied"
    );
    assert!(dir.join("routes/alice.json").exists());
    assert!(web.reconcile().await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// One HTTPS request through `curl`, which can be told where a name lives and not to
/// trust Caddy's local CA: `(status, headers lower-cased, body)`.
fn https_get(port: &str, host: &str, path: &str) -> (u16, String, String) {
    let out = Command::new("curl")
        .args([
            "-sk",
            "--max-time",
            "10",
            "--resolve",
            &format!("{host}:{port}:127.0.0.1"),
            "-D",
            "-",
            &format!("https://{host}:{port}{path}"),
        ])
        .output()
        .expect("curl");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head.to_ascii_lowercase(), body.to_owned())
}

#[tokio::test]
#[ignore = "needs a Caddy with HTTP and HTTPS ports (see the file header)"]
async fn force_https_redirects_plain_requests_and_hsts_is_sent_over_https_only() {
    let (Some(admin), Some(url), Some(tls_port)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_URL"),
        env("BUILTPANEL_TEST_CADDY_TLS_PORT"),
    ) else {
        eprintln!("skipped: no Caddy (see the script)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-web-https-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin,
            listen: vec![":18080".into(), ":18443".into()],
            automatic_https: true,
            data_dir: None,
            waf_dir: None,
            panel: None,
        },
        &dir,
    );
    let secure = |hosts: &[&str], force: bool, hsts: bool| {
        let mut s = site(hosts, Some("bp-alice-web:8080"), vec![]);
        s.force_https = force;
        s.hsts = hsts;
        s
    };
    set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![
                secure(&["forced.example.com"], true, true),
                secure(&["redirect-only.example.com"], true, false),
                secure(&["open.example.com"], false, false),
            ],
        ),
    )
    .await
    .unwrap();
    // Certificates are made when the first visitor arrives: give Caddy a moment.
    let mut first = (0, String::new(), String::new());
    for _ in 0..30 {
        first = https_get(&tls_port, "forced.example.com", "/");
        if first.0 == 200 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(first.0, 200, "{first:?}");
    assert_eq!(first.2.trim(), "hello from alice");
    // Over HTTPS the header is there, with a year.
    assert!(
        first
            .1
            .contains("strict-transport-security: max-age=31536000"),
        "{}",
        first.1
    );

    // Over plain HTTP the visitor is sent to the same address over HTTPS, method kept (308).
    let (status, location, _) = get(&url, "forced.example.com", "/some/page").await;
    assert_eq!(
        (status, location.as_deref()),
        (308, Some("https://forced.example.com/some/page"))
    );
    let (status, location, _) = get(&url, "redirect-only.example.com", "/x?y=1").await;
    assert_eq!(
        (status, location.as_deref()),
        (308, Some("https://redirect-only.example.com/x?y=1"))
    );
    // Redirect without HSTS: no header over HTTPS.
    let mut second = (0, String::new(), String::new());
    for _ in 0..30 {
        second = https_get(&tls_port, "redirect-only.example.com", "/");
        if second.0 == 200 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(second.0, 200, "{second:?}");
    assert!(
        !second.1.contains("strict-transport-security"),
        "{}",
        second.1
    );
    // A domain that did not ask is served over plain HTTP as before, never redirected.
    let (status, location, body) = get(&url, "open.example.com", "/").await;
    assert_eq!(
        (status, location, body.trim()),
        (200, None, "hello from alice")
    );

    // Turning it off takes effect on the next load.
    set_site_routes(
        web.clone(),
        set("alice", vec![secure(&["forced.example.com"], false, false)]),
    )
    .await
    .unwrap();
    assert_eq!(
        get(&url, "forced.example.com", "/").await.0,
        200,
        "no redirect any more"
    );
    let (status, headers, _) = https_get(&tls_port, "forced.example.com", "/");
    assert_eq!(status, 200);
    assert!(!headers.contains("strict-transport-security"), "{headers}");
    set_site_routes(web.clone(), set("alice", vec![]))
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The issuer line of the certificate Caddy shows for `host`.
fn served_issuer(port: &str, host: &str) -> String {
    let out = Command::new("curl")
        .args([
            "-svk",
            "--max-time",
            "10",
            "--resolve",
            &format!("{host}:{port}:127.0.0.1"),
            "-o",
            "/dev/null",
            &format!("https://{host}:{port}/"),
        ])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find(|l| l.trim_start().starts_with("issuer:") || l.contains(" issuer: "))
        .unwrap_or("")
        .to_owned()
}

#[tokio::test]
#[ignore = "needs a Caddy with HTTP and HTTPS ports (see the file header)"]
async fn an_uploaded_certificate_is_served_until_it_is_removed() {
    let (Some(admin), Some(tls_port)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_TLS_PORT"),
    ) else {
        eprintln!("skipped: no Caddy (see the script)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-web-certs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin,
            listen: vec![":18080".into(), ":18443".into()],
            automatic_https: true,
            data_dir: None,
            waf_dir: None,
            panel: None,
        },
        &dir,
    );
    // A certificate from a CA of our own, for a name Caddy would otherwise sign itself.
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Uploaded Test Root");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["own.example.com".to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let item = CustomCertificate {
        host: "own.example.com".into(),
        certificate: format!("{}{}", leaf.pem(), ca_cert.pem()),
        private_key: Some(leaf_key.serialize_pem()),
    };
    set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(
                &["own.example.com", "auto.example.com"],
                Some("bp-alice-web:8080"),
                vec![],
            )],
        ),
    )
    .await
    .unwrap();
    let done = set_custom_certificates(
        web.clone(),
        Operation::SetCustomCertificates {
            username: "alice".into(),
            items: vec![item.clone()],
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        done,
        Output::CustomCertificatesSet {
            count: 1,
            applied: true
        }
    ));
    // The key is on disk where only the agent can read it, and not with the routes.
    {
        use std::os::unix::fs::PermissionsExt as _;
        let file = dir.join("certs/alice.json");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.join("certs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(
            !std::fs::read_to_string(dir.join("routes/alice.json"))
                .unwrap()
                .contains("PRIVATE KEY")
        );
    }
    let mut issuer_line = String::new();
    for _ in 0..20 {
        issuer_line = served_issuer(&tls_port, "own.example.com");
        if issuer_line.contains("Uploaded Test Root") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(issuer_line.contains("Uploaded Test Root"), "{issuer_line}");
    // The other name still gets one of Caddy's own.
    let mut other = String::new();
    for _ in 0..30 {
        other = served_issuer(&tls_port, "auto.example.com");
        if other.contains("Caddy Local") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(other.contains("Caddy Local"), "{other}");
    // Sending the same again changes nothing.
    assert!(
        !web.reconcile().await.unwrap(),
        "nothing left to change after the certificate is in"
    );
    // A certificate without a key, or for another name, is refused before it is kept.
    for bad in [
        CustomCertificate {
            private_key: None,
            ..item.clone()
        },
        CustomCertificate {
            host: "elsewhere.example.com".into(),
            ..item.clone()
        },
    ] {
        let err = set_custom_certificates(
            web.clone(),
            Operation::SetCustomCertificates {
                username: "alice".into(),
                items: vec![bad],
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments);
        assert!(!err.message.contains("PRIVATE KEY"), "{}", err.message);
    }
    // Removing it gives the name back to Caddy.
    set_custom_certificates(
        web.clone(),
        Operation::SetCustomCertificates {
            username: "alice".into(),
            items: vec![],
        },
    )
    .await
    .unwrap();
    assert!(!dir.join("certs/alice.json").exists());
    let mut after = String::new();
    for _ in 0..30 {
        after = served_issuer(&tls_port, "own.example.com");
        if after.contains("Caddy Local") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(after.contains("Caddy Local"), "{after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "needs a Caddy and a site behind it (see the file header)"]
async fn error_pages_are_the_installations_until_an_administrator_replaces_them() {
    let (Some(admin), Some(url)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_URL"),
    ) else {
        eprintln!("skipped: no Caddy (see the script)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-web-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin,
            listen: vec![":18080".into(), ":18443".into()],
            automatic_https: false,
            data_dir: None,
            waf_dir: None,
            panel: None,
        },
        &dir,
    );
    let mut by_domain = site(&["domain-off.example.com"], None, vec![]);
    by_domain.suspended_by = Some("domain".into());
    let mut by_account = site(&["account-off.example.com"], None, vec![]);
    by_account.suspended_by = Some("account".into());
    let broken = site(&["broken.example.com"], Some("bp-alice-gone:8080"), vec![]);
    set_site_routes(
        web.clone(),
        set("alice", vec![by_domain, by_account, broken]),
    )
    .await
    .unwrap();

    // Out of the box: plain pages that differ where they should.
    let (status, _, domain_page) = get(&url, "domain-off.example.com", "/").await;
    assert_eq!(status, 503);
    assert!(
        domain_page.contains("This site is suspended"),
        "{domain_page}"
    );
    let (_, _, account_page) = get(&url, "account-off.example.com", "/").await;
    assert!(
        account_page.contains("This account is suspended"),
        "{account_page}"
    );
    let (status, _, nothing) = get(&url, "nobody.example.com", "/").await;
    assert_eq!(status, 404);
    assert!(nothing.contains("Nothing here"));
    let (status, _, down) = get(&url, "broken.example.com", "/").await;
    assert_eq!(status, 502, "{down}");
    assert!(down.contains("This site is not answering"), "{down}");

    // An administrator's pages replace them, kind by kind.
    let page = |kind: &str, html: &str| ErrorPage {
        kind: kind.into(),
        html: html.into(),
    };
    let done = set_error_pages(
        web.clone(),
        Operation::SetErrorPages {
            pages: vec![
                page("suspended_domain", "<p>domain paused</p>"),
                page("not_found", "<p>no such site</p>"),
                page("bad_gateway", "<p>back soon</p>"),
            ],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        done,
        Output::ErrorPagesSet {
            custom: 3,
            applied: true
        }
    );
    let (_, _, text) = get(&url, "domain-off.example.com", "/").await;
    assert_eq!(text, "<p>domain paused</p>");
    let (_, _, text) = get(&url, "account-off.example.com", "/").await;
    assert!(
        text.contains("This account is suspended"),
        "not replaced: {text}"
    );
    let (status, _, text) = get(&url, "nobody.example.com", "/").await;
    assert_eq!((status, text.as_str()), (404, "<p>no such site</p>"));
    let (status, _, text) = get(&url, "broken.example.com", "/").await;
    assert_eq!((status, text.as_str()), (502, "<p>back soon</p>"));
    // A working site is not touched.
    let (status, _, text) = get(&url, "x.example.com", "/").await;
    assert_eq!(status, 404);
    assert_eq!(text, "<p>no such site</p>");

    // Bad pages and kinds are refused; an empty list goes back to the plain pages.
    for bad in [
        vec![page("mystery", "<p>x</p>")],
        vec![page("not_found", "")],
        vec![page("not_found", "<p>a</p>"), page("not_found", "<p>b</p>")],
        vec![page("not_found", &"x".repeat(20_000))],
    ] {
        let err = set_error_pages(web.clone(), Operation::SetErrorPages { pages: bad })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments);
    }
    set_error_pages(web.clone(), Operation::SetErrorPages { pages: vec![] })
        .await
        .unwrap();
    let (_, _, text) = get(&url, "nobody.example.com", "/").await;
    assert!(text.contains("Nothing here"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "needs the Caddy with the firewall (CADDY_IMAGE=bp-caddy-waf:test deploy/dev/web-live.sh; see the file header)"]
async fn a_policy_change_reaches_a_running_caddy_without_a_restart() {
    let (Some(admin), Some(url), Some(waf_dir)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_URL"),
        env("BUILTPANEL_TEST_WAF_DIR").filter(|d| !d.is_empty()),
    ) else {
        eprintln!("skipped: no Caddy with the firewall");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-waf-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = WebContext::new(
        Settings {
            admin: admin.clone(),
            listen: vec![":18080".into()],
            automatic_https: false,
            data_dir: None,
            waf_dir: Some(waf_dir.into()),
            panel: None,
        },
        &dir,
    );
    let with = |waf: Option<builtpanel_node_protocol::WafSetting>| {
        let mut route = site(&["example.com"], Some("bp-alice-web:8080"), Vec::new());
        route.waf = waf;
        set_site_routes(web.clone(), set("alice", vec![route]))
    };
    let setting = |mode: &str, rules: &[u32]| {
        Some(builtpanel_node_protocol::WafSetting {
            mode: mode.into(),
            disabled_rules: rules.to_vec(),
        })
    };
    let attack = "/?id=1%27%20OR%20%271%27%3D%271";
    // No policy: everything passes, the attack too.
    with(None).await.unwrap();
    assert_eq!(get(&url, "example.com", attack).await.0, 200);
    // Block: the attack is refused, an ordinary request is not. Caddy was not restarted.
    with(setting("block", &[])).await.unwrap();
    assert_eq!(get(&url, "example.com", attack).await.0, 403);
    assert_eq!(get(&url, "example.com", "/?page=2").await.0, 200);
    // Detect: it passes (and is logged).
    with(setting("detect", &[])).await.unwrap();
    assert_eq!(get(&url, "example.com", attack).await.0, 200);
    // Rules switched off reach the loaded configuration.
    with(setting("block", &[942100, 942130])).await.unwrap();
    let config = web_config(&admin).await;
    assert!(
        config.contains("SecRuleRemoveById 942100") && config.contains("SecRuleRemoveById 942130"),
        "{config}"
    );
    // Back to none: nothing of the firewall is left in the configuration.
    with(None).await.unwrap();
    assert!(!web_config(&admin).await.contains("SecRule"));
    assert_eq!(get(&url, "example.com", attack).await.0, 200);
    // A setting that is not valid is refused and changes nothing.
    for bad in [
        setting("maybe", &[]),
        setting("block", &[0]),
        setting("block", &[1_000_000]),
    ] {
        let e = with(bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments);
    }
}

async fn web_config(admin: &str) -> String {
    let url = format!("{admin}/config/");
    tokio::task::spawn_blocking(move || {
        ureq::get(&url)
            .call()
            .unwrap()
            .body_mut()
            .read_to_string()
            .unwrap()
    })
    .await
    .unwrap()
}

/// A request with a chosen `User-Agent`: the status.
async fn get_agent(url: &str, host: &str, agent: &str) -> u16 {
    let (url, host, agent) = (url.to_owned(), host.to_owned(), agent.to_owned());
    tokio::task::spawn_blocking(move || {
        let client: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(agent)
            .build()
            .into();
        client
            .get(&format!("{url}/"))
            .header("Host", &host)
            .call()
            .unwrap()
            .status()
            .as_u16()
    })
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs a Caddy and a site behind it (see the file header)"]
async fn a_listed_user_agent_is_refused_before_any_site_and_the_list_is_kept() {
    let (Some(admin), Some(url)) = (
        env("BUILTPANEL_TEST_CADDY_ADMIN"),
        env("BUILTPANEL_TEST_CADDY_URL"),
    ) else {
        eprintln!("skipped: no Caddy (see the script)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bp-ua-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let settings = || Settings {
        admin: admin.clone(),
        listen: vec![":18080".into()],
        automatic_https: false,
        data_dir: None,
        waf_dir: None,
        panel: None,
    };
    let web = WebContext::new(settings(), &dir);
    set_site_routes(
        web.clone(),
        set(
            "alice",
            vec![site(
                &["example.com"],
                Some("bp-alice-web:8080"),
                Vec::new(),
            )],
        ),
    )
    .await
    .unwrap();
    let list = |patterns: &[&str]| Operation::SetUserAgentBlocklist {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
    };
    assert_eq!(get_agent(&url, "example.com", "SomeBadBot/2.0").await, 200);
    let out = set_user_agent_blocklist(web.clone(), list(&["badbot", "evil.scanner"]))
        .await
        .unwrap();
    assert_eq!(
        out,
        Output::UserAgentBlocklistSet {
            count: 2,
            applied: true
        }
    );
    // Refused, whatever the case; ordinary browsers and tools are not; a dot is a dot.
    assert_eq!(get_agent(&url, "example.com", "SomeBadBot/2.0").await, 403);
    assert_eq!(
        get_agent(&url, "example.com", "Mozilla evil.scanner v1").await,
        403
    );
    assert_eq!(
        get_agent(&url, "example.com", "Mozilla evilXscanner").await,
        200
    );
    assert_eq!(get_agent(&url, "example.com", "curl/8.5.0").await, 200);
    // For every name Caddy serves, not only one site; the 403 is plain.
    assert_eq!(get_agent(&url, "unknown.example.org", "badbot").await, 403);
    // Kept: a new context (an agent that restarted) puts it back on a Caddy that forgot it.
    let again = WebContext::new(settings(), &dir);
    again.reconcile().await.unwrap();
    assert_eq!(get_agent(&url, "example.com", "badbot").await, 403);
    // Taken off the list: it passes again.
    set_user_agent_blocklist(web.clone(), list(&["evil.scanner"]))
        .await
        .unwrap();
    assert_eq!(get_agent(&url, "example.com", "badbot").await, 200);
    set_user_agent_blocklist(web.clone(), list(&[]))
        .await
        .unwrap();
    assert_eq!(get_agent(&url, "example.com", "evil.scanner").await, 200);
    // Bad lists are refused and change nothing.
    set_user_agent_blocklist(web.clone(), list(&["badbot"]))
        .await
        .unwrap();
    let too_long = "x".repeat(101);
    for bad in [
        list(&[""]),
        list(&["tab\there"]),
        list(&[too_long.as_str()]),
        list(&["a", "A"]),
    ] {
        let e = set_user_agent_blocklist(web.clone(), bad)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments);
    }
    assert_eq!(get_agent(&url, "example.com", "badbot").await, 403);
    set_user_agent_blocklist(web.clone(), list(&[]))
        .await
        .unwrap();
}
