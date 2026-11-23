//! `read_log` for real (T10.04): containers named as an account's web and PHP containers are,
//! Docker's rootless daemon, the agent's handler. Ignored by default; needs Docker.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerManager, ImageRef};
use builtpanel_node_agent::ops::{logs::read_log, tenant::TenantContext};
use builtpanel_node_protocol::{ErrorCode, LogLine, LogSource, Operation, Output};
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

async fn read(
    context: &Arc<TenantContext>,
    username: &str,
    source: LogSource,
    since_ms: i64,
    tail: u32,
    contains: Option<&str>,
) -> Result<(Vec<LogLine>, bool), builtpanel_node_protocol::AgentError> {
    let out = read_log(
        context.clone(),
        Operation::ReadLog {
            tenant_id: Uuid::nil(),
            username: username.into(),
            source,
            since_ms,
            tail,
            contains: contains.map(str::to_owned),
        },
    )
    .await?;
    match out {
        Output::LogLines { lines, truncated } => Ok((lines, truncated)),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn an_accounts_logs_are_read_by_source_time_and_search_and_only_its_own() {
    let docker = DockerManager::connect(&docker_socket()).unwrap();
    let image = ImageRef::parse(
        "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .unwrap();
    docker.pull_image(&image).await.unwrap();
    let username = format!("lg{}x", std::process::id());
    let tenant = builtpanel_domain::TenantUsername::parse(&username).unwrap();
    let web = builtpanel_tenant_stack::web_container_name(&tenant).unwrap();
    let php = builtpanel_tenant_stack::php_container_name(&tenant).unwrap();
    for (name, script) in [
        (
            &web,
            "i=0; while [ $i -lt 40 ]; do echo \"GET /page-$i 200\"; echo \"[error] failed $i, host: \\\"example.com\\\"\" >&2; i=$((i+1)); done; sleep 600",
        ),
        (
            &php,
            "echo 'PHP Warning: something' >&2; echo 'plain output'; sleep 600",
        ),
    ] {
        let _ = docker.remove(name, true).await;
        let mut spec = ContainerSpec::new(name.clone(), image.clone());
        spec.command = vec!["sh".into(), "-c".into(), script.into()];
        docker.create_container(&spec).await.unwrap();
        docker.start(name).await.unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let base = std::env::temp_dir().join(format!("bp-logs-live-{}", std::process::id()));
    let context = Arc::new(TenantContext {
        helper_socket: base.join("none.sock"),
        tenants_root: base.join("tenants"),
        docker: docker.clone(),
        vhosts_root: base.join("vhosts"),
    });

    // Standard output is the access log, standard error the error log.
    let (access, _) = read(&context, &username, LogSource::WebAccess, 0, 100, None)
        .await
        .unwrap();
    assert_eq!(access.len(), 40);
    assert!(
        access.iter().all(|l| l.text.starts_with("GET /page-"))
            && access[0].at_ms > 1_700_000_000_000
    );
    assert!(access.windows(2).all(|w| w[0].at_ms <= w[1].at_ms));
    let (errors, _) = read(&context, &username, LogSource::WebError, 0, 100, None)
        .await
        .unwrap();
    assert_eq!(errors.len(), 40);
    assert!(errors.iter().all(|l| l.text.contains("example.com")));
    // The newest lines when there are more than asked for.
    let (last, truncated) = read(&context, &username, LogSource::WebAccess, 0, 5, None)
        .await
        .unwrap();
    assert_eq!(last.len(), 5);
    assert!(
        last[4].text.ends_with("39 200") || last[4].text.contains("page-39"),
        "{last:?}"
    );
    assert!(truncated);
    // A search, case-insensitive, over what is there.
    let (found, _) = read(
        &context,
        &username,
        LogSource::WebError,
        0,
        100,
        Some("FAILED 3"),
    )
    .await
    .unwrap();
    let texts: Vec<&str> = found.iter().map(|l| l.text.as_str()).collect();
    assert!(
        texts.iter().all(|t| t.contains("failed 3")) && texts.len() == 11,
        "{texts:?}"
    );
    // Asking from after the last line gives nothing new: the live view's cursor.
    let newest = access.last().unwrap().at_ms;
    let (none, _) = read(
        &context,
        &username,
        LogSource::WebAccess,
        newest + 5000,
        100,
        None,
    )
    .await
    .unwrap();
    assert!(none.is_empty(), "{none:?}");
    // PHP's errors, not its output.
    let (php_errors, _) = read(&context, &username, LogSource::PhpError, 0, 100, None)
        .await
        .unwrap();
    assert_eq!(php_errors.len(), 1);
    assert_eq!(php_errors[0].text, "PHP Warning: something");

    // Bad arguments; another account's containers are not there to be read.
    for (tail, since) in [(0u32, 0i64), (5000, 0), (10, -1)] {
        let e = read(&context, &username, LogSource::WebAccess, since, tail, None)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{tail} {since}");
    }
    let e = read(&context, "../etc", LogSource::WebAccess, 0, 10, None)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);
    assert!(
        read(
            &context,
            &format!("{username}z"),
            LogSource::WebAccess,
            0,
            10,
            None
        )
        .await
        .is_err()
    );

    docker.remove(&web, true).await.unwrap();
    docker.remove(&php, true).await.unwrap();
}
