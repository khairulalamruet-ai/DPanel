//! The node's firewall (T11.06): one nftables table of its own, `inet builtpanel_fw`.
//!
//! The ruleset is written from a fixed template out of a checked [`FirewallPolicy`]
//! (addresses and ports only; nothing else of the request reaches the text), tested with
//! `nft -c`, and then loaded in one transaction (`delete table` and the new table in the same
//! file), so a ruleset the packet filter refuses leaves the old table as it was.
//!
//! What is never cut off, whatever the policy says: answers to connections the node made
//! itself, loopback, ICMP, the SSH port (the helper's own setting) and the `always_allow`
//! addresses. The table hooks in at priority -5, after the database port's table (-10).

use std::{
    io::Write as _,
    process::{Command, Stdio},
};

use builtpanel_node_protocol::{FirewallPolicy, PortRule};

use crate::{protocol::ErrorCode, tenant::Failure};

const NFT: &str = "/usr/sbin/nft";
/// The table the helper owns.
pub const TABLE: &str = "builtpanel_fw";

fn set_text(name: &str, cidrs: &[String]) -> String {
    let elements = if cidrs.is_empty() {
        String::new()
    } else {
        format!("        elements = {{ {} }}\n", cidrs.join(", "))
    };
    format!(
        "    set {name} {{\n        type ipv4_addr\n        flags interval\n        auto-merge\n{elements}    }}\n"
    )
}

fn port_text(rule: &PortRule) -> String {
    match rule.to {
        Some(to) if to != rule.port => format!("{}-{to}", rule.port),
        _ => rule.port.to_string(),
    }
}

/// The whole ruleset for a checked policy and the SSH port.
#[must_use]
pub fn ruleset(policy: &FirewallPolicy, ssh_port: u16) -> String {
    let mut text =
        format!("table inet {TABLE}\ndelete table inet {TABLE}\ntable inet {TABLE} {{\n");
    text.push_str(&set_text("always", &policy.always_allow));
    text.push_str(&set_text("allowed", &policy.allowed));
    text.push_str(&set_text("blocked", &policy.blocked));
    text.push_str(
        "    chain input {\n        type filter hook input priority -5; policy accept;\n",
    );
    text.push_str("        ct state established,related accept\n");
    text.push_str("        iifname \"lo\" accept\n");
    text.push_str("        ip saddr @always accept\n");
    text.push_str(&format!("        tcp dport {ssh_port} accept\n"));
    text.push_str("        ip protocol icmp accept\n");
    text.push_str("        ip6 nexthdr icmpv6 accept\n");
    text.push_str("        ip saddr @allowed accept\n");
    text.push_str("        ip saddr @blocked drop\n");
    for (i, limit) in policy.rate_limits.iter().enumerate() {
        text.push_str(&format!(
            "        {proto} dport {port} ct state new meter rate{i} {{ ip saddr limit rate over {n}/minute }} drop\n",
            proto = limit.proto,
            port = limit.port,
            n = limit.per_minute
        ));
    }
    for rule in &policy.open_ports {
        let from = if rule.from.is_empty() {
            String::new()
        } else {
            format!("ip saddr {{ {} }} ", rule.from.join(", "))
        };
        text.push_str(&format!(
            "        {from}{} dport {} accept\n",
            rule.proto,
            port_text(rule)
        ));
    }
    if policy.default_deny {
        text.push_str("        meta nfproto ipv4 counter drop\n");
    }
    text.push_str("    }\n}\n");
    text
}

/// How many rules the chain of a policy has (what the answer reports).
#[must_use]
pub fn rule_count(policy: &FirewallPolicy) -> usize {
    8 + policy.rate_limits.len() + policy.open_ports.len() + usize::from(policy.default_deny)
}

fn run(script: &str, check_only: bool) -> Result<(), Failure> {
    let failed = |_: std::io::Error| (ErrorCode::Failed, "cannot run the packet filter tool");
    let mut command = Command::new(NFT);
    if check_only {
        command.arg("-c");
    }
    let mut child = command
        .args(["-f", "-"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(failed)?;
    child
        .stdin
        .take()
        .ok_or((ErrorCode::Failed, "no input for the packet filter tool"))?
        .write_all(script.as_bytes())
        .map_err(failed)?;
    if child.wait().map_err(failed)?.success() {
        Ok(())
    } else {
        Err((ErrorCode::Failed, "the packet filter refused the rules"))
    }
}

/// Loads a ruleset text: tested first, then applied; a refusal changes nothing.
///
/// # Errors
/// The packet filter refusing the text, or not running.
pub fn apply(script: &str) -> Result<(), Failure> {
    run(script, true)?;
    run(script, false)
}

/// Puts the node firewall in place.
///
/// # Errors
/// A policy that is not valid (not retryable), or the packet filter failing.
pub fn set(policy: &FirewallPolicy, ssh_port: u16) -> Result<(), Failure> {
    let policy = policy.checked().map_err(|_| {
        (
            ErrorCode::InvalidArguments,
            "the firewall policy is not valid",
        )
    })?;
    apply(&ruleset(&policy, ssh_port))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use builtpanel_node_protocol::RateLimit;

    #[test]
    fn the_ruleset_keeps_ssh_the_panel_and_answers_whatever_the_policy_says() {
        let policy = FirewallPolicy {
            default_deny: true,
            always_allow: vec!["203.0.113.7/32".into()],
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
                    port: 21100,
                    to: Some(21119),
                    proto: "tcp".into(),
                    from: vec!["10.0.0.0/8".into()],
                },
            ],
            rate_limits: vec![RateLimit {
                port: 443,
                proto: "tcp".into(),
                per_minute: 120,
            }],
        };
        let text = ruleset(&policy.checked().unwrap(), 2200);
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing in\n{text}"))
        };
        // The safe things come before anything that can drop, and the final drop is last.
        assert!(at("ct state established,related accept") < at("ip saddr @blocked drop"));
        assert!(at("ip saddr @always accept") < at("ip saddr @blocked drop"));
        assert!(at("tcp dport 2200 accept") < at("ip saddr @blocked drop"));
        assert!(at("ip saddr @blocked drop") < at("tcp dport 80 accept"));
        assert!(at("tcp dport 80 accept") < at("counter drop"));
        assert!(text.contains("ip saddr { 10.0.0.0/8 } tcp dport 21100-21119 accept"));
        assert!(text.contains(
            "tcp dport 443 ct state new meter rate0 { ip saddr limit rate over 120/minute } drop"
        ));
        assert!(text.contains("elements = { 203.0.113.7/32 }"));
        // Without default_deny nothing drops what is not named.
        let open = ruleset(&FirewallPolicy::default(), 22);
        assert!(!open.contains("counter drop"));
    }

    #[test]
    fn a_policy_is_checked_and_made_plain() {
        let ok = FirewallPolicy {
            blocked: vec!["192.0.2.1".into(), "192.0.2.1/32".into()],
            ..FirewallPolicy::default()
        }
        .checked()
        .unwrap();
        assert_eq!(ok.blocked, ["192.0.2.1/32"]);
        for bad in [
            FirewallPolicy {
                default_deny: true,
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                blocked: vec!["0.0.0.0/0".into()],
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                blocked: vec!["1.1.1.1; flush ruleset".into()],
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                open_ports: vec![PortRule {
                    port: 0,
                    to: None,
                    proto: "tcp".into(),
                    from: vec![],
                }],
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                open_ports: vec![PortRule {
                    port: 90,
                    to: Some(80),
                    proto: "tcp".into(),
                    from: vec![],
                }],
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                open_ports: vec![PortRule {
                    port: 80,
                    to: None,
                    proto: "icmp".into(),
                    from: vec![],
                }],
                ..FirewallPolicy::default()
            },
            FirewallPolicy {
                rate_limits: vec![RateLimit {
                    port: 80,
                    proto: "tcp".into(),
                    per_minute: 0,
                }],
                ..FirewallPolicy::default()
            },
        ] {
            assert!(bad.checked().is_err(), "{bad:?}");
        }
        assert_eq!(
            set(
                &FirewallPolicy {
                    default_deny: true,
                    ..FirewallPolicy::default()
                },
                22
            )
            .unwrap_err()
            .0,
            ErrorCode::InvalidArguments
        );
    }
}
