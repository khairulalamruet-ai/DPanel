//! The node firewall for real (T11.06). Two parts, started by deploy/dev/firewall-live.sh:
//! the agent's handler through the privileged helper (root, started by the script; as the login
//! user), and the packet filter's own check of a deny-everything ruleset (as root, nothing is
//! loaded). Ignored by default.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::Write as _,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
};

use builtpanel_docker_manager::DockerManager;
use builtpanel_node_agent::ops::{server::set_firewall_rules, tenant::TenantContext};
use builtpanel_node_protocol::{ErrorCode, FirewallPolicy, Operation, Output, PortRule, RateLimit};
use builtpanel_priv_helper::nodefw;

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

fn listing() -> String {
    // Readable by the root part only; the helper part reads it through the script.
    String::from_utf8_lossy(
        &Command::new("/usr/sbin/nft")
            .args(["list", "table", "inet", "builtpanel_fw"])
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default(),
    )
    .into_owned()
}

#[tokio::test]
#[ignore = "needs the privileged helper running as root"]
async fn a_policy_reaches_the_packet_filter_and_a_bad_one_changes_nothing() {
    let Some(socket) = std::env::var_os("BUILTPANEL_TEST_HELPER_SOCKET") else {
        eprintln!("skipped: no helper (see the live script)");
        return;
    };
    let context = Arc::new(TenantContext {
        helper_socket: PathBuf::from(socket),
        tenants_root: std::env::temp_dir(),
        docker: DockerManager::connect(&docker_socket()).unwrap(),
        vhosts_root: std::env::temp_dir().join("vhosts"),
    });
    let ask = |policy: FirewallPolicy| {
        let context = context.clone();
        async move { set_firewall_rules(context, Operation::SetFirewallRules { policy }).await }
    };
    // Something harmless on this machine: an address from the documentation range is blocked,
    // another allowed, a port nobody uses is open and rate limited.
    let good = FirewallPolicy {
        default_deny: false,
        always_allow: vec!["203.0.113.250".into()],
        allowed: vec!["198.51.100.0/24".into()],
        blocked: vec!["192.0.2.0/24".into(), "192.0.2.77".into()],
        open_ports: vec![PortRule {
            port: 65001,
            to: Some(65003),
            proto: "tcp".into(),
            from: vec![],
        }],
        rate_limits: vec![RateLimit {
            port: 65001,
            proto: "tcp".into(),
            per_minute: 30,
        }],
    };
    let out = ask(good).await.unwrap();
    assert!(
        matches!(out, Output::FirewallSet { rules } if rules >= 10),
        "{out:?}"
    );
    // A policy that is not valid is refused here, before the helper (and the table is not touched).
    for bad in [
        FirewallPolicy {
            default_deny: true,
            ..FirewallPolicy::default()
        },
        FirewallPolicy {
            blocked: vec!["not an address".into()],
            ..FirewallPolicy::default()
        },
        FirewallPolicy {
            blocked: vec!["0.0.0.0/0".into()],
            ..FirewallPolicy::default()
        },
    ] {
        let e = ask(bad).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArguments, "{e:?}");
    }
    // Replaced, not added to.
    ask(FirewallPolicy {
        blocked: vec!["192.0.2.99".into()],
        ..FirewallPolicy::default()
    })
    .await
    .unwrap();
}

/// As root: what the helper loads, and what happens to the old table when a ruleset is refused.
#[test]
#[ignore = "needs root and nft"]
fn the_packet_filter_accepts_every_shape_of_policy_and_keeps_the_old_table_when_it_refuses() {
    if std::env::var_os("BUILTPANEL_TEST_NFT_ROOT").is_none() {
        eprintln!("skipped: not the root part");
        return;
    }
    let deny = FirewallPolicy {
        default_deny: true,
        always_allow: vec!["203.0.113.250".into()],
        allowed: vec!["198.51.100.0/24".into()],
        blocked: vec!["192.0.2.0/24".into()],
        open_ports: vec![
            PortRule {
                port: 80,
                to: None,
                proto: "tcp".into(),
                from: vec![],
            },
            PortRule {
                port: 443,
                to: None,
                proto: "tcp".into(),
                from: vec!["10.0.0.0/8".into()],
            },
            PortRule {
                port: 21100,
                to: Some(21119),
                proto: "tcp".into(),
                from: vec![],
            },
            PortRule {
                port: 53,
                to: None,
                proto: "udp".into(),
                from: vec![],
            },
        ],
        rate_limits: vec![
            RateLimit {
                port: 443,
                proto: "tcp".into(),
                per_minute: 120,
            },
            RateLimit {
                port: 53,
                proto: "udp".into(),
                per_minute: 600,
            },
        ],
    }
    .checked()
    .unwrap();
    // The deny-everything shape is only checked (-c): nothing is loaded.
    let script = nodefw::ruleset(&deny, 22);
    let mut child = Command::new("/usr/sbin/nft")
        .args(["-c", "-f", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let checked = child.wait_with_output().unwrap();
    assert!(
        checked.status.success(),
        "{}\n{script}",
        String::from_utf8_lossy(&checked.stderr)
    );
    // The table the helper loaded earlier is there with the replaced content; a refused ruleset
    // (a rule the filter does not know) leaves it exactly as it was.
    let before = listing();
    assert!(before.contains("192.0.2.99"), "{before}");
    let broken = "table inet builtpanel_fw\\ndelete table inet builtpanel_fw\\ntable inet builtpanel_fw {\\n chain input { type filter hook input priority -5; policy accept; this is not a rule }\\n}\\n";
    assert!(nodefw::apply(&broken.replace("\\\\n", "\\n")).is_err());
    assert_eq!(listing(), before, "the old table stays");
}
