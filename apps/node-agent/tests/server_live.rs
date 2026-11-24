//! The server tools for real (T10.15): the privileged helper (root, started by the live script
//! with its server tools run directly and a stand-in for the reboot), the agent's handlers.
//! Ignored by default; skips without BUILTPANEL_TEST_HELPER_SOCKET and BUILTPANEL_TEST_SWAP_FILE.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{
    server::{
        add_ssh_key, drop_caches, drop_swap, read_ssh_access, reboot_node, remove_ssh_key,
        report_server_info, set_ssh_settings, set_swap, set_timezone, write_sshd_config,
    },
    tenant::TenantContext,
};
use builtpanel_node_protocol::{ErrorCode, Operation, Output, ServerInfo, SshAccess};

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

    // Swap goes empty and comes back on (when there is memory for what is in it).
    match drop_swap(context.clone(), Operation::DropSwap).await {
        Ok(out) => {
            assert!(matches!(out, Output::ServerActionDone { ref action } if action == "drop_swap"))
        }
        Err(e) => assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}"),
    }
    // The cache, and the reboot (answered first; the helper is asked a few seconds later, and
    // the live script gave it a stand-in command).
    let dropped = drop_caches(context.clone(), Operation::DropCaches)
        .await
        .unwrap();
    assert!(matches!(dropped, Output::ServerActionDone { ref action } if action == "drop_caches"));
    let rebooted = reboot_node(context.clone(), Operation::RebootNode { hard: false })
        .await
        .unwrap();
    assert!(matches!(rebooted, Output::ServerActionDone { ref action } if action == "reboot_node"));
    tokio::time::sleep(Duration::from_secs(5)).await;
}

async fn ssh(context: &Arc<TenantContext>) -> SshAccess {
    match read_ssh_access(context.clone(), Operation::ReadSshAccess)
        .await
        .unwrap()
    {
        Output::SshAccess { access } => access,
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the privileged helper running as root, with a real sshd and a folder of its own"]
async fn ssh_access_is_read_and_changed_with_a_real_sshd_and_never_into_a_lock_out() {
    let (Some(socket), Some(key), Some(fingerprint), Some(dropin)) = (
        std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET"),
        std::env::var("BUILTPANEL_TEST_SSH_KEY").ok(),
        std::env::var("BUILTPANEL_TEST_SSH_FINGERPRINT").ok(),
        std::env::var_os("BUILTPANEL_TEST_SSH_DROPIN"),
    ) else {
        eprintln!("skipped: no helper with an ssh folder (see the live script)");
        return;
    };
    let context = Arc::new(TenantContext {
        helper_socket: PathBuf::from(socket),
        tenants_root: std::env::temp_dir(),
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: std::env::temp_dir().join("vhosts"),
    });
    let dropin = PathBuf::from(dropin);
    let invalid = |e: builtpanel_node_protocol::AgentError| e.code == ErrorCode::InvalidArguments;

    // What is there: the folder's own sshd_config (no Port line: 22), no keys yet.
    let start = ssh(&context).await;
    assert_eq!(start.ports, vec![22]);
    assert!(start.keys.is_empty() && start.password_authentication && start.pubkey_authentication);

    // A key goes in, is listed under the fingerprint ssh-keygen prints, and not twice.
    add_ssh_key(context.clone(), Operation::AddSshKey { key: key.clone() })
        .await
        .unwrap();
    let keys = ssh(&context).await.keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].fingerprint, fingerprint);
    assert!(invalid(
        add_ssh_key(context.clone(), Operation::AddSshKey { key })
            .await
            .unwrap_err()
    ));
    for bad in [
        "",
        "ssh-ed25519 notakey",
        "command=\"x\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMq",
    ] {
        assert!(invalid(
            add_ssh_key(context.clone(), Operation::AddSshKey { key: bad.into() })
                .await
                .unwrap_err()
        ));
    }

    // The port, the root login and password login change; sshd -T reports them back.
    set_ssh_settings(
        context.clone(),
        Operation::SetSshSettings {
            port: Some(2291),
            root_login: Some("prohibit-password".into()),
            password_authentication: Some(false),
            pubkey_authentication: None,
        },
    )
    .await
    .unwrap();
    let now = ssh(&context).await;
    assert_eq!(now.ports, vec![2291]);
    assert_eq!(now.root_login, "prohibit-password");
    assert!(!now.password_authentication && now.pubkey_authentication);
    // The drop-in is the helper's file (root's folder): read it when this user may.
    if let Ok(text) = std::fs::read_to_string(&dropin) {
        assert!(text.contains("Port 2291"), "{text}");
    }

    // Nobody could log in after these two: refused, and nothing changed.
    let e = set_ssh_settings(
        context.clone(),
        Operation::SetSshSettings {
            port: None,
            root_login: None,
            password_authentication: None,
            pubkey_authentication: Some(false),
        },
    )
    .await
    .unwrap_err();
    assert!(
        e.code == ErrorCode::InvalidArguments && e.message.contains("lock everyone out"),
        "{e:?}"
    );
    let e = remove_ssh_key(
        context.clone(),
        Operation::RemoveSshKey {
            fingerprint: fingerprint.clone(),
        },
    )
    .await
    .unwrap_err();
    assert!(e.message.contains("lock everyone out"), "{e:?}");
    assert_eq!(ssh(&context).await.keys.len(), 1);
    // A port of the panel, a port below 1024, a choice that does not exist, nothing at all.
    for bad in [
        Operation::SetSshSettings {
            port: Some(8444),
            root_login: None,
            password_authentication: None,
            pubkey_authentication: None,
        },
        Operation::SetSshSettings {
            port: Some(80),
            root_login: None,
            password_authentication: None,
            pubkey_authentication: None,
        },
        Operation::SetSshSettings {
            port: None,
            root_login: Some("maybe".into()),
            password_authentication: None,
            pubkey_authentication: None,
        },
        Operation::SetSshSettings {
            port: None,
            root_login: None,
            password_authentication: None,
            pubkey_authentication: None,
        },
    ] {
        assert!(invalid(
            set_ssh_settings(context.clone(), bad).await.unwrap_err()
        ));
    }

    // With passwords back on, the key can go (and an unknown one cannot).
    set_ssh_settings(
        context.clone(),
        Operation::SetSshSettings {
            port: None,
            root_login: None,
            password_authentication: Some(true),
            pubkey_authentication: None,
        },
    )
    .await
    .unwrap();
    assert!(invalid(
        remove_ssh_key(
            context.clone(),
            Operation::RemoveSshKey {
                fingerprint: "SHA256:nope".into()
            }
        )
        .await
        .unwrap_err()
    ));
    remove_ssh_key(context.clone(), Operation::RemoveSshKey { fingerprint })
        .await
        .unwrap();
    assert!(ssh(&context).await.keys.is_empty());

    // The whole file: sshd -t rejects nonsense and the file stays; a good text goes in.
    let e = write_sshd_config(
        context.clone(),
        Operation::WriteSshdConfig {
            text: "ThisIsNotAnSshdSetting yes\n".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(invalid(e));
    assert_eq!(ssh(&context).await.ports, vec![2291]);
    write_sshd_config(
        context.clone(),
        Operation::WriteSshdConfig {
            text: std::env::var("BUILTPANEL_TEST_SSH_NEWCONFIG").unwrap(),
        },
    )
    .await
    .unwrap();
    assert!(ssh(&context).await.ports.contains(&2292));
}
