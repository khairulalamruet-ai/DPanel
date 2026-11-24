//! Deploying from a git repository for real (T12.09): a repository served over http inside a
//! tenant-style network, fetched, built in a Node.js sandbox and switched in atomically.
//! Needs rootless Docker, git and the network; ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, Name, Network, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_agent::ops::{
    gitdeploy::{remove_git_deploy, run_git_deploy},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, files_volume_name, network_name};
use uuid::Uuid;

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

fn open_dir(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Commits `version` and publishes the repository as a dumb-http bare repository.
fn publish(work: &Path, srv: &Path, version: &str) {
    std::fs::write(work.join("VERSION"), format!("{version}\n")).unwrap();
    std::fs::write(work.join("index.html"), format!("<p>{version}</p>\n")).unwrap();
    git(work, &["add", "-A"]);
    git(work, &["commit", "-q", "-m", version]);
    let bare = srv.join("site.git");
    let _ = std::fs::remove_dir_all(&bare);
    git(
        srv,
        &["clone", "-q", "--bare", work.to_str().unwrap(), "site.git"],
    );
    git(&bare, &["update-server-info"]);
    // Readable by the container that serves it.
    Command::new("chmod")
        .args(["-R", "a+rX", srv.to_str().unwrap()])
        .status()
        .unwrap();
}

#[tokio::test]
#[ignore = "needs rootless Docker, git and the network"]
async fn a_repository_is_fetched_built_and_switched_in_atomically() {
    let base = std::env::temp_dir().join(format!("bp-gitdeploy-live-{}", Uuid::now_v7().simple()));
    let _ = std::fs::remove_dir_all(&base);
    let tenants = base.join("tenants");
    let state = base.join("state");
    std::fs::create_dir_all(state.join("vhosts")).unwrap();
    let tenant = Uuid::now_v7();
    let www = tenants.join(tenant.to_string()).join("www");
    open_dir(&www);
    let work = base.join("work");
    let srv = base.join("srv");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&srv).unwrap();
    git(&work, &["init", "-q"]);
    publish(&work, &srv, "v1");
    Command::new("chmod")
        .args(["a+rx", base.to_str().unwrap()])
        .status()
        .unwrap();

    let username = TenantUsername::parse("alice").unwrap();
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let network = network_name(&username).unwrap();
    let files = files_volume_name(&username).unwrap();
    for n in ["bp-alice-gitsrv"] {
        let _ = docker.remove(&Name::parse(n).unwrap(), true).await;
    }
    let _ = docker.remove_volume(&files).await;
    let _ = docker
        .remove_volume(&Name::parse("bp-alice-gitsrv-data").unwrap())
        .await;
    docker.ensure_network(&network, false).await.unwrap();
    docker
        .create_bind_volume(&files, &www, &tenants)
        .await
        .unwrap();
    // The repository's server: Python's own http server on the tenant's network.
    let allowlist = Allowlist::embedded().unwrap();
    let data = Name::parse("bp-alice-gitsrv-data").unwrap();
    docker.create_bind_volume(&data, &srv, &base).await.unwrap();
    let image = allowlist
        .runtime_image(Role::Python, "3.13")
        .unwrap()
        .clone();
    docker.pull_image(&image).await.unwrap();
    let mut server = ContainerSpec::new(Name::parse("bp-alice-gitsrv").unwrap(), image);
    server.entrypoint = vec![
        "python".into(),
        "-m".into(),
        "http.server".into(),
        "8000".into(),
        "--directory".into(),
        "/srv".into(),
    ];
    server.network = Network::Named(network.clone());
    server.volumes = vec![VolumeMount {
        volume: data,
        target: "/srv".into(),
        read_only: true,
    }];
    docker.create_container(&server).await.unwrap();
    docker
        .start(&Name::parse("bp-alice-gitsrv").unwrap())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let context = Arc::new(TenantContext {
        helper_socket: base.join("no-helper.sock"),
        tenants_root: tenants,
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: state.join("vhosts"),
    });
    let deploy =
        |repo: &str, branch: &str, dest: &str, steps: &[&[&str]]| Operation::RunGitDeploy {
            deploy_id: Uuid::now_v7(),
            tenant_id: tenant,
            username: "alice".into(),
            uid: 10_000,
            name: "site".into(),
            repo_url: repo.into(),
            branch: branch.into(),
            dest: dest.into(),
            build_kind: (!steps.is_empty()).then(|| "node".into()),
            build_version: (!steps.is_empty()).then(|| "24".into()),
            build_steps: steps
                .iter()
                .map(|s| s.iter().map(|w| (*w).to_owned()).collect())
                .collect(),
            restart_app: None,
            token: None,
        };
    let repo = "http://bp-alice-gitsrv:8000/site.git";
    let build: &[&[&str]] = &[&[
        "node",
        "-e",
        "const fs = require('fs'); fs.writeFileSync('built.txt', 'built ' + fs.readFileSync('VERSION', 'utf8').trim())",
    ]];

    // Bad input.
    for bad in [
        deploy(repo, "--upload-pack=x", "site", &[]),
        deploy("git@github.com:a/b.git", "main", "site", &[]),
        deploy("file:///etc", "main", "site", &[]),
        deploy(repo, "main", "../outside", &[]),
        deploy(repo, "main", ".bp-releases/x", &[]),
        deploy(repo, "main", "site", &[&[]]),
    ] {
        let err = run_git_deploy(context.clone(), bad).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{err:?}");
    }

    // The first deploy: fetched, built, switched in.
    let out = run_git_deploy(context.clone(), deploy(repo, "main", "site", build))
        .await
        .unwrap();
    let Output::GitDeployed { commit, log } = out else {
        panic!("{out:?}")
    };
    assert_eq!(commit.len(), 40);
    assert!(
        log.contains("git clone") && log.contains("node -e"),
        "{log}"
    );
    let first = std::fs::read_link(www.join("site")).unwrap();
    assert!(first.starts_with(".bp-releases/site/"), "{first:?}");
    assert_eq!(
        std::fs::read_to_string(www.join("site/built.txt")).unwrap(),
        "built v1"
    );
    assert_eq!(
        std::fs::read_to_string(www.join("site/index.html")).unwrap(),
        "<p>v1</p>\n"
    );
    assert!(!www.join("site/.git").exists(), "the history is not kept");

    // A new commit: the link moves to a new release in one step; the old release stays.
    publish(&work, &srv, "v2");
    let out = run_git_deploy(context.clone(), deploy(repo, "main", "site", build))
        .await
        .unwrap();
    let Output::GitDeployed {
        commit: second_commit,
        ..
    } = out
    else {
        panic!()
    };
    assert_ne!(second_commit, commit);
    let second = std::fs::read_link(www.join("site")).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        std::fs::read_to_string(www.join("site/built.txt")).unwrap(),
        "built v2"
    );
    assert!(
        www.join(&first).exists(),
        "the old release is kept for a rollback"
    );

    // A build that fails leaves what is live alone and leaves no release behind.
    let releases = |www: &Path| {
        std::fs::read_dir(www.join(".bp-releases/site"))
            .unwrap()
            .count()
    };
    let before = releases(&www);
    let err = run_git_deploy(
        context.clone(),
        deploy(
            repo,
            "main",
            "site",
            &[&["node", "-e", "console.error('boom here'); process.exit(3)"]],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        err.message.contains("build step") && err.message.contains("boom here"),
        "{err:?}"
    );
    assert!(!err.retryable);
    assert_eq!(std::fs::read_link(www.join("site")).unwrap(), second);
    assert_eq!(releases(&www), before, "the failed release is removed");
    // A repository or a branch that is not there.
    let err = run_git_deploy(
        context.clone(),
        deploy("http://bp-alice-gitsrv:8000/none.git", "main", "site", &[]),
    )
    .await
    .unwrap_err();
    assert!(
        err.message.contains("fetching the repository failed"),
        "{err:?}"
    );
    let err = run_git_deploy(context.clone(), deploy(repo, "no-such-branch", "site", &[]))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("fetching the repository failed"),
        "{err:?}"
    );
    assert_eq!(std::fs::read_link(www.join("site")).unwrap(), second);

    // A folder that is not a link made by a deploy is not replaced.
    std::fs::create_dir_all(www.join("real")).unwrap();
    open_dir(&www.join("real"));
    let err = run_git_deploy(context.clone(), deploy(repo, "main", "real", &[]))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("is not a link made by a deploy"),
        "{err:?}"
    );
    assert!(www.join("real").is_dir() && !www.join("real").is_symlink());

    // Below a folder: the link is relative to its own folder.
    let out = run_git_deploy(context.clone(), deploy(repo, "main", "apps/site", &[]))
        .await
        .unwrap();
    assert!(matches!(out, Output::GitDeployed { .. }));
    assert!(
        std::fs::read_link(www.join("apps/site"))
            .unwrap()
            .starts_with("../.bp-releases/")
    );
    assert_eq!(
        std::fs::read_to_string(www.join("apps/site/index.html")).unwrap(),
        "<p>v2</p>\n"
    );

    // Many deploys: only the last few releases are kept (and the one in use is among them).
    for i in 0..5 {
        publish(&work, &srv, &format!("v{}", 3 + i));
        run_git_deploy(context.clone(), deploy(repo, "main", "site", &[]))
            .await
            .unwrap();
    }
    assert!(releases(&www) <= 4 + 1, "{}", releases(&www));
    assert_eq!(
        std::fs::read_to_string(www.join("site/index.html")).unwrap(),
        "<p>v7</p>\n"
    );

    // Removing the deploy keeps the files.
    let out = remove_git_deploy(
        context.clone(),
        Operation::RemoveGitDeploy {
            tenant_id: tenant,
            username: "alice".into(),
            name: "site".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(out, Output::GitDeployRemoved);
    assert!(www.join("site/index.html").exists());
    let _ = docker
        .remove(&Name::parse("bp-alice-gitsrv").unwrap(), true)
        .await;
}
