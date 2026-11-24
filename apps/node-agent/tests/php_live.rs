//! The PHP versions of a tenant for real (T06.09): the main PHP container changes
//! version, another version runs next to it for a site that asks for it, and the
//! web server reaches each through its own configuration. Ignored by default;
//! needs the login user's rootless Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{
    ContainerSpec, DockerManager, ImageRef, Name, Network, VolumeMount,
};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    tenant::{TenantContext, provision_tenant_stack, set_php_versions},
    vhost::attach_domain,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{conf_volume_name, files_volume_name, network_name};
use uuid::Uuid;

const BUSYBOX: &str =
    "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

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

/// Runs a shell snippet in a throw-away container on the tenant's network, with
/// the files volume at /data (as root of the user namespace); returns its exit code.
async fn run(docker: &DockerManager, owner: &TenantUsername, n: u32, script: &str) -> i64 {
    let mut spec = ContainerSpec::new(
        Name::parse(&format!("bp-{owner}-run{n}")).unwrap(),
        ImageRef::parse(BUSYBOX).unwrap(),
    );
    spec.command = vec!["sh".into(), "-c".into(), script.into()];
    spec.network = Network::Named(network_name(owner).unwrap());
    spec.volumes.push(VolumeMount {
        volume: files_volume_name(owner).unwrap(),
        target: "/data".into(),
        read_only: false,
    });
    // (Another test pulling the same image at the same moment may make one pull fail.)
    for attempt in 0..4 {
        if docker.pull_image(&spec.image).await.is_ok() || attempt == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    docker.create_container(&spec).await.unwrap();
    docker.start(&spec.name).await.unwrap();
    let mut exit = -1;
    for _ in 0..300 {
        let info = docker.inspect(&spec.name).await.unwrap();
        if !info.running && info.status != "created" {
            exit = info.exit_code.unwrap_or(-1);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    docker.remove(&spec.name, true).await.unwrap();
    exit
}

/// Whether the site `host` answers with a PHP version starting with `prefix`.
async fn serves(
    docker: &DockerManager,
    owner: &TenantUsername,
    n: u32,
    host: &str,
    prefix: &str,
) -> bool {
    for attempt in 0..20 {
        let script = format!(
            "wget -q -O - --header 'Host: {host}' http://bp-{owner}-web:8080/index.php | grep -q '^{prefix}'"
        );
        if run(docker, owner, n * 100 + attempt, &script).await == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn the_main_php_changes_version_another_runs_beside_it_and_extras_are_removed() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    let root = std::env::temp_dir().join(format!("bp-php-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let context = Arc::new(TenantContext {
        helper_socket: "/nonexistent".into(),
        tenants_root: "/nonexistent".into(),
        docker: docker.clone(),
        vhosts_root: root.clone(),
    });
    let name = format!("q{}x", std::process::id());
    let owner = TenantUsername::parse(&name).unwrap();
    let uid = 10_089;
    let tenant_id = Uuid::now_v7();
    let limits = |php_version: &str, extra: &[&str]| Operation::SetPhpVersions {
        tenant_id,
        username: name.clone(),
        uid,
        cpu_millis: Some(1000),
        memory_bytes: Some(768 << 20),
        pids: Some(256),
        php_version: php_version.into(),
        extra_php: extra.iter().map(|v| (*v).to_owned()).collect(),
    };

    // What create_tenant makes (the helper part is not needed here): the network
    // and the files volume, with a page that says which PHP runs it.
    docker
        .ensure_network(&network_name(&owner).unwrap(), false)
        .await
        .unwrap();
    docker
        .create_volume(&files_volume_name(&owner).unwrap())
        .await
        .unwrap();
    let seeded = run(
        &docker,
        &owner,
        1,
        "echo '<?php echo PHP_VERSION;' > /data/index.php && echo '<?php echo ini_get(\"memory_limit\"), \" \", ini_get(\"max_execution_time\"), \" \", ini_get(\"date.timezone\");' > /data/ini.php && chmod 755 /data && chmod 644 /data/index.php /data/ini.php",
    )
    .await;
    assert_eq!(seeded, 0);

    // A new stack runs the version new accounts start with; two sites on it.
    let done = provision_tenant_stack(
        context.clone(),
        Operation::ProvisionTenantStack {
            tenant_id,
            username: name.clone(),
            uid,
            cpu_millis: Some(1000),
            memory_bytes: Some(768 << 20),
            pids: Some(256),
        },
    )
    .await
    .unwrap();
    assert!(matches!(done, Output::StackProvisioned { .. }));
    let site = |domain: &str, php: &str| Operation::AttachDomain {
        username: name.clone(),
        domain: domain.into(),
        config: format!(
            "server {{\n    listen 8080;\n    server_name {domain};\n    root /var/www/html;\n    location ~ \\.php$ {{\n        include fastcgi_params;\n        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n        fastcgi_pass {php};\n    }}\n}}\n"
        ),
    };
    attach_domain(
        context.clone(),
        site("one.example.com", &format!("bp-{name}-php:9000")),
    )
    .await
    .unwrap();
    assert!(serves(&docker, &owner, 2, "one.example.com", "8.4.").await);

    // php.ini options reach the running container (and survive a change of version).
    let seen = |what: &'static str| {
        let (docker, owner) = (docker.clone(), owner.clone());
        async move {
            for attempt in 0..20 {
                let script = format!(
                    "wget -q -O - --header 'Host: one.example.com' http://bp-{owner}-web:8080/ini.php | grep -q '^{what}$'"
                );
                if run(&docker, &owner, 700 + attempt, &script).await == 0 {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            false
        }
    };
    assert!(seen("128M 30 UTC").await, "the image defaults");
    let ini = |text: &str| Operation::SetPhpIni {
        username: name.clone(),
        ini: text.into(),
    };
    let set = builtpanel_node_agent::ops::vhost::set_php_ini(
        context.clone(),
        ini("memory_limit = 96M\nmax_execution_time = 45\ndate.timezone = Asia/Dhaka\n"),
    )
    .await
    .unwrap();
    assert!(matches!(set, Output::PhpIniSet { reloaded: 1 }), "{set:?}");
    assert!(
        seen("96M 45 Asia/Dhaka").await,
        "the running container loaded it"
    );
    // A file with an option that is not on the list is refused and changes nothing.
    let err = builtpanel_node_agent::ops::vhost::set_php_ini(
        context.clone(),
        ini("memory_limit = 32M\nauto_prepend_file = /etc/passwd\n"),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    assert!(seen("96M 45 Asia/Dhaka").await);

    // Restarting the web server or PHP keeps the site up afterwards.
    for service in ["web", "php"] {
        let done = builtpanel_node_agent::ops::tenant::restart_service(
            context.clone(),
            Operation::RestartService {
                username: name.clone(),
                service: service.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            done,
            Output::ServiceRestarted {
                containers: vec![format!("bp-{name}-{service}")]
            }
        );
        assert!(
            serves(&docker, &owner, 9, "one.example.com", "8.4.").await,
            "{service}"
        );
    }
    let err = builtpanel_node_agent::ops::tenant::restart_service(
        context.clone(),
        Operation::RestartService {
            username: name.clone(),
            service: "mail".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);

    // The main container moves to 8.3: its files stay, the site follows. A visitor asks the site
    // ten times a second while it happens: the site may be without PHP for a moment, never for
    // five seconds (T11.12).
    let visitor = Name::parse(&format!("bp-{owner}-visitor")).unwrap();
    let watching = {
        let (docker, owner, visitor) = (docker.clone(), owner.clone(), visitor.clone());
        tokio::spawn(async move {
            let mut spec = ContainerSpec::new(visitor, ImageRef::parse(BUSYBOX).unwrap());
            spec.command = vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "i=0; while [ $i -lt 1500 ]; do if wget -q -T 1 -O /dev/null --header 'Host: one.example.com' http://bp-{owner}-web:8080/index.php; then echo up; else echo down; fi; i=$((i+1)); sleep 0.1; done"
                ),
            ];
            spec.network = Network::Named(network_name(&owner).unwrap());
            docker
                .run_to_end(&spec, Duration::from_secs(300))
                .await
                .unwrap()
                .output
        })
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    let moved = set_php_versions(context.clone(), limits("8.3", &[]))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    docker.stop(&visitor, Duration::from_secs(1)).await.unwrap();
    let seen_by_visitor = watching.await.unwrap();
    let (mut longest, mut run, mut ups) = (0, 0, 0);
    for line in seen_by_visitor.lines() {
        if line.trim() == "down" {
            run += 1;
            longest = longest.max(run);
        } else if line.trim() == "up" {
            run = 0;
            ups += 1;
        }
    }
    eprintln!("visitor: {ups} answers, longest run of failures: {longest}");
    assert!(ups > 20, "the visitor really watched: {seen_by_visitor}");
    // A failed request takes up to a second (the visitor's timeout) and a tenth more to repeat.
    assert!(
        f64::from(longest) * 1.1 < 5.0,
        "the site was without PHP for about {} seconds",
        f64::from(longest) * 1.1
    );
    assert_eq!(
        moved,
        Output::PhpVersionsSet {
            containers: vec![format!("bp-{name}-php")],
            removed: vec![]
        }
    );
    assert!(serves(&docker, &owner, 3, "one.example.com", "8.3.").await);
    assert!(
        seen("96M 45 Asia/Dhaka").await,
        "the options follow the account to the new version"
    );
    // Saying it again changes nothing.
    assert_eq!(
        set_php_versions(context.clone(), limits("8.3", &[]))
            .await
            .unwrap(),
        moved
    );

    // A second site on another version gets a container of its own; the first is
    // untouched and the plan's PHP share is divided between the two.
    attach_domain(
        context.clone(),
        site("two.example.com", &format!("bp-{name}-php-8-2:9000")),
    )
    .await
    .err();
    let both = set_php_versions(context.clone(), limits("8.3", &["8.2"]))
        .await
        .unwrap();
    assert_eq!(
        both,
        Output::PhpVersionsSet {
            containers: vec![format!("bp-{name}-php"), format!("bp-{name}-php-8-2")],
            removed: vec![]
        }
    );
    attach_domain(
        context.clone(),
        site("two.example.com", &format!("bp-{name}-php-8-2:9000")),
    )
    .await
    .unwrap();
    assert!(serves(&docker, &owner, 4, "two.example.com", "8.2.").await);
    assert!(serves(&docker, &owner, 5, "one.example.com", "8.3.").await);
    let main = docker
        .inspect(&Name::parse(&format!("bp-{name}-php")).unwrap())
        .await
        .unwrap();
    let extra = docker
        .inspect(&Name::parse(&format!("bp-{name}-php-8-2")).unwrap())
        .await
        .unwrap();
    assert_eq!(main.limits.memory_bytes, extra.limits.memory_bytes);
    assert_eq!(main.limits.memory_bytes, (768u64 << 20) * 75 / 100 / 2);

    // A version nobody offers, and a stack that was never made, are refused.
    let err = set_php_versions(context.clone(), limits("7.4", &[]))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    let err = set_php_versions(
        context.clone(),
        Operation::SetPhpVersions {
            tenant_id,
            username: format!("{name}z"),
            uid,
            cpu_millis: None,
            memory_bytes: None,
            pids: None,
            php_version: "8.4".into(),
            extra_php: vec![],
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);

    // The version nobody uses any more is removed; the other site goes on.
    let back = set_php_versions(context.clone(), limits("8.3", &[]))
        .await
        .unwrap();
    assert_eq!(
        back,
        Output::PhpVersionsSet {
            containers: vec![format!("bp-{name}-php")],
            removed: vec![format!("bp-{name}-php-8-2")]
        }
    );
    assert!(
        docker
            .inspect(&Name::parse(&format!("bp-{name}-php-8-2")).unwrap())
            .await
            .is_err()
    );
    assert!(serves(&docker, &owner, 6, "one.example.com", "8.3.").await);

    // Clean up.
    for container in [format!("bp-{name}-web"), format!("bp-{name}-php")] {
        let _ = docker.remove(&Name::parse(&container).unwrap(), true).await;
    }
    let _ = docker
        .remove_volume(&conf_volume_name(&owner).unwrap())
        .await;
    let _ = docker
        .remove_volume(&files_volume_name(&owner).unwrap())
        .await;
    let _ = docker.remove_network(&network_name(&owner).unwrap()).await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[ignore = "needs rootless Docker (see the file header)"]
async fn an_image_that_cannot_be_pulled_gives_way_to_the_next() {
    let socket = docker_socket();
    if !socket.exists() {
        eprintln!("skipped: no Docker at {}", socket.display());
        return;
    }
    let docker = DockerManager::connect(&socket).unwrap();
    // A made-up registry name (it never resolves) first, a real image second.
    let missing = ImageRef::parse(&format!(
        "registry.invalid/builtpanel/none@sha256:{}",
        "0".repeat(64)
    ))
    .unwrap();
    let real = ImageRef::parse(BUSYBOX).unwrap();
    let chosen = builtpanel_node_agent::ops::tenant::pull_first(&docker, &[&missing, &real])
        .await
        .unwrap();
    assert_eq!(chosen, 1);
    assert_eq!(
        builtpanel_node_agent::ops::tenant::pull_first(&docker, &[&real, &missing])
            .await
            .unwrap(),
        0
    );
    let err = builtpanel_node_agent::ops::tenant::pull_first(&docker, &[&missing])
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);
}
