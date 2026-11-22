//! A domain's page cache against a real nginx and a real PHP-FPM started from the
//! stack template (T12.11): a page is kept and given again, a visitor with a login
//! cookie is never served from it, and a purge makes the next request fresh.
//! Ignored by default; needs the login user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

use builtpanel_docker_manager::{DockerManager, PublishedPort};
use builtpanel_domain::{TenantUsername, UidRange};
use builtpanel_node_agent::ops::{
    tenant::TenantContext,
    vhost::{attach_domain, conf_dir},
};
use builtpanel_node_protocol::{Operation, Output};
use builtpanel_tenant_stack::{
    Allowlist, PlanResources, Tenant, conf_volume_name, files_volume_name, network_name, render,
};
use uuid::Uuid;

const PORT: u16 = 18_092;

fn docker_socket() -> PathBuf {
    // The engine the live tests run against: another one (Kiri) than the rootless Docker of this user.
    if let Some(path) = std::env::var_os("BUILTPANEL_TEST_DOCKER_SOCKET") {
        return PathBuf::from(path);
    }
    let uid = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .unwrap();
    PathBuf::from(format!("/run/user/{uid}/docker.sock"))
}

/// What the site answers on the published port: `(curl exit code, body)`.
fn fetch_path(path: &str, cookie: Option<&str>) -> (String, String) {
    let url = format!("http://127.0.0.1:{PORT}{path}");
    let mut args = vec!["-s", "-i", "--max-time", "5"];
    if let Some(c) = cookie {
        args.extend(["-H", c]);
    }
    args.push(&url);
    let out = Command::new("curl").args(args).output().expect("curl");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("x-cache-status:"))
        .map(|l| l.split_once(':').unwrap().1.trim().to_owned())
        .unwrap_or_default();
    (status, body.trim().to_owned())
}

fn fetch(cookie: Option<&str>) -> (String, String) {
    fetch_path("/", cookie)
}

/// Waits until the site answers with a page at all.
async fn answers() -> bool {
    for _ in 0..50 {
        if !fetch_path("/warm-up", None).1.is_empty() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn a_cached_page_is_given_again_except_to_signed_in_visitors_and_a_purge_makes_it_fresh() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-pcache-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.clone(),
    });
    let username = format!("c{}x", std::process::id());
    let owner = TenantUsername::parse(&username).unwrap();
    let uid = UidRange::DEFAULT.allocate(10_090..10_091).unwrap();
    let list = Allowlist::embedded().unwrap();
    let stack = render(
        &Tenant {
            id: Uuid::now_v7(),
            username: &owner,
            uid,
            resources: PlanResources {
                cpu_millis: Some(500),
                memory_bytes: Some(512 << 20),
                pids: Some(128),
            },
            php_version: None,
            extra_php: &[],
        },
        &list,
    )
    .unwrap();
    let mut web = None;
    let mut php = None;
    for s in stack.services {
        match s.role.as_deref() {
            Some("web") => web = Some(s),
            Some("php") => php = Some(s),
            _ => {}
        }
    }
    let (mut web, php) = (web.unwrap(), php.unwrap());
    web.ports = vec![PublishedPort {
        container_port: 8080,
        host_port: PORT,
        host_ip: "127.0.0.1".parse().unwrap(),
    }];
    docker
        .ensure_network(&network_name(&owner).unwrap(), false)
        .await
        .unwrap();
    docker
        .create_volume(&files_volume_name(&owner).unwrap())
        .await
        .unwrap();
    let dir = conf_dir(&root, &owner).unwrap();
    docker
        .create_bind_volume(&conf_volume_name(&owner).unwrap(), &dir, &root)
        .await
        .unwrap();
    // The PHP container also reads its options from a directory of the tenant.
    let ini = builtpanel_node_agent::ops::vhost::php_ini_dir(&root, &owner).unwrap();
    docker
        .create_bind_volume(
            &builtpanel_tenant_stack::php_ini_volume_name(&owner).unwrap(),
            &ini,
            &root,
        )
        .await
        .unwrap();
    docker.pull_image(&web.image).await.unwrap();
    docker.pull_image(&php.image).await.unwrap();
    docker.create_container(&php).await.unwrap();
    docker.start(&php.name).await.unwrap();
    docker.create_container(&web).await.unwrap();
    docker.start(&web.name).await.unwrap();

    // A page that is different every time it is made.
    let write = Command::new("docker")
        .env("DOCKER_HOST", format!("unix://{}", socket.display()))
        .args([
            "exec",
            "-u",
            "0",
            php.name.as_str(),
            "sh",
            "-c",
            "printf '<?php header(\"Cache-Control: public\"); echo uniqid(\"page-\", true);' > /var/www/html/index.php",
        ])
        .output()
        .unwrap();
    assert!(write.status.success(), "{write:?}");

    let attach = |generation: u32| {
        let policy = builtpanel_domain::vhost::Policy::default().with_page_cache(Some(
            builtpanel_domain::vhost::PageCache::parse(5, generation).unwrap(),
        ));
        let domain = builtpanel_domain::Fqdn::parse("example.com").unwrap();
        let config = builtpanel_domain::vhost::render(
            builtpanel_domain::vhost::DEFAULT_NGINX_TEMPLATE,
            &builtpanel_domain::vhost::Site {
                domain: &domain,
                aliases: &[],
                docroot: &builtpanel_domain::Docroot::default(),
                php_upstream: &builtpanel_tenant_stack::php_upstream(&owner).unwrap(),
                policy: &policy,
                custom: "",
                proxy: None,
            },
        )
        .unwrap();
        Operation::AttachDomain {
            username: username.clone(),
            domain: "example.com".into(),
            config,
        }
    };
    let done = attach_domain(context.clone(), attach(1)).await.unwrap();
    assert_eq!(done, Output::DomainAttached { applied: true });
    assert!(answers().await, "the site never answered");

    // The first visit makes the page, the second is given from the cache.
    let (first_status, first) = fetch(None);
    let (second_status, second) = fetch(None);
    assert!(first.starts_with("page-"), "{first:?}");
    assert_eq!(first_status, "MISS", "{first:?}");
    assert_eq!(second_status, "HIT");
    assert_eq!(first, second, "the cached page is the same page");

    // A signed-in visitor is never served from the cache, and what they see is not kept.
    let (status, mine) = fetch(Some("Cookie: wordpress_logged_in_abc=1"));
    assert_eq!(status, "BYPASS");
    assert_ne!(mine, first);
    let (third_status, third) = fetch(None);
    assert_eq!(
        third, first,
        "the visitor's page did not replace the cached one (status {third_status})"
    );

    // A purge: the next visit makes the page again, and it is cached again.
    attach_domain(context.clone(), attach(2)).await.unwrap();
    // nginx finishes the requests of its old workers first, so the new key takes
    // a moment to be the one that is used.
    let mut fresh = String::new();
    let mut status = String::new();
    for _ in 0..30 {
        (status, fresh) = fetch(None);
        if status == "MISS" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(status, "MISS");
    assert_ne!(fresh, first);
    let (status, again) = fetch(None);
    assert_eq!(status, "HIT");
    assert_eq!(again, fresh);

    // Clean up.
    let _ = docker.remove(&web.name, true).await;
    let _ = docker.remove(&php.name, true).await;
    for v in [
        conf_volume_name(&owner).unwrap(),
        files_volume_name(&owner).unwrap(),
        builtpanel_tenant_stack::php_ini_volume_name(&owner).unwrap(),
    ] {
        let _ = docker.remove_volume(&v).await;
    }
    let _ = docker.remove_network(&network_name(&owner).unwrap()).await;
    let _ = std::fs::remove_dir_all(&root);
}
