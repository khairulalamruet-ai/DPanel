//! The malware scan for real (T11.09): ClamAV in containers (needs Docker, the network for the
//! image and the signatures, and a few minutes the first time). Ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    scan::{scan_tenant, update_malware_signatures},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
use uuid::Uuid;

const EICAR: &str = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

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

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn a_planted_test_file_is_found_and_clean_files_and_folders_are_not() {
    // A fixed place: the signatures (owned by the scanner's user, so not ours to remove) are
    // kept between runs, which saves fetching them again.
    let base = std::env::temp_dir().join("bp-scan-live");
    let tenants = base.join("tenants");
    let _ = std::fs::remove_dir_all(&tenants);
    let state = base.join("state");
    std::fs::create_dir_all(&tenants).unwrap();
    std::fs::create_dir_all(state.join("vhosts")).unwrap();
    let id = Uuid::now_v7();
    let www = tenants.join(id.to_string()).join("www");
    std::fs::create_dir_all(www.join("clean")).unwrap();
    std::fs::write(www.join("eicar.com"), EICAR).unwrap();
    std::fs::write(www.join("index.html"), "<p>hello</p>").unwrap();
    std::fs::write(www.join("clean").join("a.txt"), "fine").unwrap();
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let _ = docker
        .remove_volume(&builtpanel_docker_manager::Name::parse("bp-clamav-db").unwrap())
        .await;
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: tenants.clone(),
        docker,
        vhosts_root: state.join("vhosts"),
    });
    let scan = |folder: Option<&str>| {
        let context = context.clone();
        let op = Operation::ScanTenant {
            tenant_id: id,
            uid: 10_000,
            folder: folder.map(str::to_owned),
            quarantine: false,
        };
        async move { scan_tenant(context, op).await }
    };
    // The first scan fetches the signatures by itself.
    let out = scan(None).await.unwrap();
    let Output::Scanned {
        scanned,
        infected,
        more,
    } = out
    else {
        panic!("{out:?}")
    };
    assert!(!more && scanned >= 3, "{scanned}");
    assert_eq!(infected.len(), 1, "{infected:?}");
    assert_eq!(infected[0].path, "www/eicar.com");
    assert!(infected[0].signature.contains("Eicar"), "{infected:?}");
    assert!(!infected[0].quarantined);
    // A clean folder finds nothing; a folder outside the files is refused.
    let Output::Scanned { infected, .. } = scan(Some("clean")).await.unwrap() else {
        panic!()
    };
    assert!(infected.is_empty());
    for bad in ["../x", "/etc", "a/../../b"] {
        let e = scan(Some(bad)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{bad}");
    }
    // Updating again works (nothing new to fetch is fine).
    let out = update_malware_signatures(context.clone(), Operation::UpdateMalwareSignatures)
        .await
        .unwrap();
    assert!(matches!(out, Output::SignaturesUpdated { .. }), "{out:?}");
    // Nothing is left behind.
    let left = context.docker.list().await.unwrap();
    assert!(
        left.iter()
            .all(|n| !n.starts_with("bp-scan-") && n != "bp-clamav-update"),
        "{left:?}"
    );
    let _ = std::fs::remove_dir_all(&tenants);
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn a_wordpress_install_is_listed_with_its_plugins_and_themes() {
    let base = std::env::temp_dir().join("bp-scan-live-wp");
    let tenants = base.join("tenants");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("state").join("vhosts")).unwrap();
    let id = Uuid::now_v7();
    let wp = tenants.join(id.to_string()).join("www").join("blog");
    std::fs::create_dir_all(wp.join("wp-includes")).unwrap();
    std::fs::write(
        wp.join("wp-includes").join("version.php"),
        "<?php\n$wp_version = '6.4.3';\n$wp_db_version = 57155;\n",
    )
    .unwrap();
    let plugin = wp.join("wp-content").join("plugins").join("old-forms");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("old-forms.php"),
        "<?php\n/**\n * Plugin Name: Old Forms\n * Version: 1.2.3\n */\n",
    )
    .unwrap();
    std::fs::create_dir_all(wp.join("wp-content").join("plugins").join("not-a-plugin")).unwrap();
    let theme = wp.join("wp-content").join("themes").join("mytheme");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::write(
        theme.join("style.css"),
        "/*\nTheme Name: My Theme\nVersion: 2.0\n*/\n",
    )
    .unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: tenants,
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: base.join("state").join("vhosts"),
    });
    let out = builtpanel_node_agent::ops::scan::inventory_wordpress(
        context.clone(),
        Operation::InventoryWordPress {
            tenant_id: id,
            uid: 10_000,
        },
    )
    .await
    .unwrap();
    let Output::WordPressInventory { installs } = out else {
        panic!("{out:?}")
    };
    assert_eq!(installs.len(), 1, "{installs:?}");
    assert_eq!(installs[0].path, "www/blog");
    assert_eq!(installs[0].core, "6.4.3");
    assert_eq!(installs[0].plugins.len(), 1, "{installs:?}");
    assert_eq!(
        (
            installs[0].plugins[0].slug.as_str(),
            installs[0].plugins[0].version.as_str()
        ),
        ("old-forms", "1.2.3")
    );
    assert_eq!(installs[0].themes[0].version, "2.0");
    let left = context.docker.list().await.unwrap();
    assert!(left.iter().all(|n| !n.starts_with("bp-wpinv-")), "{left:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
#[ignore = "needs rootless Docker and the network"]
async fn an_old_image_has_findings_a_registry_answers_and_unlisted_images_are_refused() {
    let base = std::env::temp_dir().join("bp-scan-live-img");
    std::fs::create_dir_all(base.join("state").join("vhosts")).unwrap();
    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: base.join("tenants"),
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: base.join("state").join("vhosts"),
    });
    // An image that is not on the allow-list is refused before anything runs.
    let e = builtpanel_node_agent::ops::imagescan::scan_image(
        context.clone(),
        Operation::ScanImage {
            reference: format!("alpine@sha256:{}", "c".repeat(64)),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);
    // A registry answers for a tag (the digest of the index, as a pull would fetch it).
    let out = builtpanel_node_agent::ops::imagescan::check_image_updates(
        context.clone(),
        Operation::CheckImageUpdates {
            images: vec![
                "clamav/clamav:stable_base".into(),
                "no/such-image-x:nope".into(),
            ],
        },
    )
    .await
    .unwrap();
    let Output::ImageDigests { items } = out else {
        panic!("{out:?}")
    };
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(items[0].digest.starts_with("sha256:") && items[0].digest.len() == 71);
    // An older image of the list is scanned for real (the first run fetches Trivy's database).
    let out = builtpanel_node_agent::ops::imagescan::scan_image(
        context.clone(),
        Operation::ScanImage {
            reference: "nginxinc/nginx-unprivileged@sha256:7377697a821c131a924a7105fafbe7414db4e9fcc77a6f08f776f33f141ec3f8".into(),
        },
    )
    .await
    .unwrap();
    let Output::ImageScanned {
        critical,
        high,
        medium,
        low,
        top,
    } = out
    else {
        panic!("{out:?}")
    };
    eprintln!(
        "findings: {critical} critical, {high} high, {medium} medium, {low} low; top {}",
        top.len()
    );
    assert!(
        critical + high + medium + low > 0,
        "an old image should have findings"
    );
    assert!(!top.is_empty() && top.len() <= 50);
    assert!(top[0].id.contains('-'), "{:?}", top[0]);
    let left = context.docker.list().await.unwrap();
    assert!(left.iter().all(|n| n != "bp-trivy-scan"), "{left:?}");
    let _ = std::fs::remove_dir_all(&base);
}
