//! The server tools for real (T10.15): the privileged helper (root, started by the live script
//! with its server tools run directly and a stand-in for the reboot), the agent's handlers.
//! Ignored by default; skips without BUILTPANEL_TEST_HELPER_SOCKET and BUILTPANEL_TEST_SWAP_FILE.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    server::{drop_caches, reboot_node, report_server_info, set_swap, set_timezone},
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output, ServerInfo};

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

async fn info() -> ServerInfo {
    match report_server_info(Operation::ReportServerInfo)
        .await
        .unwrap()
    {
        Output::ServerInfo { info } => info,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the privileged helper running as root"]
async fn the_server_tools_report_and_change_the_machine() {
    let (Some(socket), Some(swap_file)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var_os("BUILTPANEL_TEST_SWAP_FILE"),
    ) else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let context = Arc::new(TenantContext {
        helper_socket: PathBuf::from(socket),
        tenants_root: std::env::temp_dir(),
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: std::env::temp_dir().join("vhosts"),
    });
    // What the node says about itself.
    let before = info().await;
    assert!(!before.hostname.is_empty() && !before.os.is_empty() && !before.kernel.is_empty());
    assert!(before.uptime_seconds > 0 && before.memory_total_bytes > before.swap_used_bytes);
    assert!(before.ssh.iter().all(|(k, _)| {
        k.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    }));

    // The time zone: a real one, then the old one back; nothing else is accepted.
    let other = if before.timezone == "Asia/Dhaka" {
        "Europe/Berlin"
    } else {
        "Asia/Dhaka"
    };
    let done = set_timezone(
        context.clone(),
        Operation::SetTimezone { zone: other.into() },
    )
    .await
    .unwrap();
    assert!(matches!(done, Output::ServerActionDone { .. }));
    assert_eq!(info().await.timezone, other);
    for bad in ["../etc/passwd", "Not/AZone", "UTC; reboot", ""] {
        let e = set_timezone(context.clone(), Operation::SetTimezone { zone: bad.into() })
            .await
            .unwrap_err();
        assert!(
            matches!(e.code, ErrorCode::InvalidArguments | ErrorCode::Internal),
            "{bad}: {e:?}"
        );
    }
    assert_eq!(
        info().await.timezone,
        other,
        "a refused zone changes nothing"
    );
    if !before.timezone.is_empty() {
        set_timezone(
            context.clone(),
            Operation::SetTimezone {
                zone: before.timezone.clone(),
            },
        )
        .await
        .unwrap();
    }

    // Swap: made, seen as swap, replaced by another size, taken away; too big is refused.
    let swap_before = info().await.swap_total_bytes;
    set_swap(context.clone(), Operation::SetSwap { megabytes: 64 })
        .await
        .unwrap();
    let with = info().await;
    // The swap area loses its first page to its header.
    let added = with.swap_total_bytes - swap_before;
    assert!(
        (64 * 1024 * 1024 - 65_536..=64 * 1024 * 1024).contains(&added),
        "{added}"
    );
    assert_eq!(
        std::fs::metadata(&swap_file).unwrap().len(),
        64 * 1024 * 1024
    );
    set_swap(context.clone(), Operation::SetSwap { megabytes: 32 })
        .await
        .unwrap();
    let added = info().await.swap_total_bytes - swap_before;
    assert!(
        (32 * 1024 * 1024 - 65_536..=32 * 1024 * 1024).contains(&added),
        "{added}"
    );
    set_swap(context.clone(), Operation::SetSwap { megabytes: 0 })
        .await
        .unwrap();
    assert_eq!(info().await.swap_total_bytes, swap_before);
    assert!(!std::path::Path::new(&swap_file).exists());
    let e = set_swap(
        context.clone(),
        Operation::SetSwap {
            megabytes: 1_000_000,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArguments);

    // The cache, and the reboot (answered first; the helper is asked a few seconds later, and
    // the live script gave it a stand-in command).
    let dropped = drop_caches(context.clone(), Operation::DropCaches)
        .await
        .unwrap();
    assert!(matches!(dropped, Output::ServerActionDone { ref action } if action == "drop_caches"));
    let rebooted = reboot_node(context.clone(), Operation::RebootNode)
        .await
        .unwrap();
    assert!(matches!(rebooted, Output::ServerActionDone { ref action } if action == "reboot_node"));
    tokio::time::sleep(Duration::from_secs(5)).await;
}
