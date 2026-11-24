//! The database port's firewall (T07.05b): the one place the helper touches the
//! node's packet filter.
//!
//! One nftables table of its own, `inet builtpanel_db`, with a set of allowed IPv4
//! addresses and ranges and a chain that lets those through to the database's
//! public port and drops everything else aimed at it. Which port is fixed when the
//! helper starts (`--db-port`): no request can name another one, so it cannot be
//! used to cut off the node's other services. The agent asks for the list of
//! allowed ranges; the helper checks each, writes the whole ruleset from a fixed
//! template and hands it to `nft` in one transaction (no shell, no environment).
//! Nothing else on the node's firewall is read, changed or removed.

use std::{
    io::Write as _,
    process::{Command, Stdio},
};

use crate::{protocol::ErrorCode, tenant::Failure};

/// Where `nft` is.
const NFT: &str = "/usr/sbin/nft";
/// The table the helper owns for the MariaDB server's public port.
pub const TABLE: &str = "builtpanel_db";
/// The table the helper owns for the PostgreSQL server's public port.
pub const TABLE_PG: &str = "builtpanel_pg";
/// Most ranges in one request.
pub const MAX_CIDRS: usize = 2000;

/// Whether `text` is an IPv4 address or range the helper will put in a rule:
/// `a.b.c.d/n`, n from 8 to 32, the start of its range.
#[must_use]
pub fn valid_cidr(text: &str) -> bool {
    let Some((ip, bits)) = text.split_once('/') else {
        return false;
    };
    let Some(bits) = bits.parse::<u32>().ok().filter(|b| (8..=32).contains(b)) else {
        return false;
    };
    if bits.to_string() != text.split_once('/').map_or("", |(_, b)| b) {
        return false;
    }
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    let mut octets = [0u8; 4];
    for (slot, part) in octets.iter_mut().zip(&parts) {
        // No sign, no leading zero, no more than three digits.
        let plain = !part.is_empty()
            && part.len() <= 3
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'));
        match part.parse::<u8>() {
            Ok(n) if plain => *slot = n,
            _ => return false,
        }
    }
    let mask = u32::MAX << (32 - bits);
    u32::from_be_bytes(octets) & !mask == 0
}

/// The whole ruleset for `port` and the allowed ranges. Only validated ranges and
/// a port number are put into the text.
#[must_use]
pub fn ruleset(port: u16, cidrs: &[String]) -> String {
    ruleset_in(TABLE, port, cidrs)
}

/// [`ruleset`] in one of the helper's two tables.
#[must_use]
pub fn ruleset_in(table: &str, port: u16, cidrs: &[String]) -> String {
    let elements = if cidrs.is_empty() {
        String::new()
    } else {
        format!("        elements = {{ {} }}\n", cidrs.join(", "))
    };
    format!(
        "table inet {table}\n\
         delete table inet {table}\n\
         table inet {table} {{\n\
         \x20   set allowed {{\n\
         \x20       type ipv4_addr\n\
         \x20       flags interval\n\
         \x20       auto-merge\n\
         {elements}\
         \x20   }}\n\
         \x20   chain input {{\n\
         \x20       type filter hook input priority -10; policy accept;\n\
         \x20       tcp dport {port} ip saddr @allowed accept\n\
         \x20       tcp dport {port} drop\n\
         \x20   }}\n\
         }}\n"
    )
}

/// Puts the ruleset in place: only these ranges reach the database port.
///
/// # Errors
/// [`Failure`] with a fixed message: a bad range, or `nft` failing.
pub fn set(port: u16, cidrs: &[String]) -> Result<(), Failure> {
    set_in(TABLE, port, cidrs)
}

/// [`set`] in one of the helper's two tables (`table` is one of [`TABLE`], [`TABLE_PG`]).
///
/// # Errors
/// [`Failure`] with a fixed message: a bad range, or `nft` failing.
pub fn set_in(table: &'static str, port: u16, cidrs: &[String]) -> Result<(), Failure> {
    if cidrs.len() > MAX_CIDRS {
        return Err((ErrorCode::InvalidArguments, "too many address ranges"));
    }
    let mut unique: Vec<String> = Vec::new();
    for cidr in cidrs {
        if !valid_cidr(cidr) {
            return Err((
                ErrorCode::InvalidArguments,
                "an address range is not IPv4 a.b.c.d/n (n from 8 to 32)",
            ));
        }
        if !unique.contains(cidr) {
            unique.push(cidr.clone());
        }
    }
    let script = ruleset_in(table, port, &unique);
    let failed = |_: std::io::Error| (ErrorCode::Failed, "cannot run the packet filter tool");
    let mut child = Command::new(NFT)
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
    let status = child.wait().map_err(failed)?;
    if status.success() {
        Ok(())
    } else {
        Err((ErrorCode::Failed, "the packet filter refused the rules"))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_ipv4_ranges_that_start_where_they_say_are_accepted() {
        for ok in [
            "203.0.113.7/32",
            "198.51.100.0/24",
            "10.0.0.0/8",
            "192.168.0.0/20",
        ] {
            assert!(valid_cidr(ok), "{ok}");
        }
        for bad in [
            "",
            "0.0.0.0/0",
            "10.0.0.0/7",
            "203.0.113.7",
            "203.0.113.7/24",
            "203.0.113.7/33",
            "203.0.113.007/32",
            "203.0.113.7/032",
            "+3.0.113.7/32",
            "203.0.113/32",
            "203.0.113.7.1/32",
            "203.0.113.256/32",
            "::1/128",
            "%",
            "203.0.113.7/32; drop",
            "203.0.113.7/32\nflush ruleset",
            "any",
            "{ 1.1.1.1 }",
        ] {
            assert!(!valid_cidr(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_ruleset_is_a_fixed_template_with_the_port_and_the_ranges() {
        let text = ruleset(13306, &["203.0.113.7/32".into(), "198.51.100.0/24".into()]);
        assert_eq!(
            text,
            "table inet builtpanel_db\n\
             delete table inet builtpanel_db\n\
             table inet builtpanel_db {\n\
             \x20   set allowed {\n\
             \x20       type ipv4_addr\n\
             \x20       flags interval\n\
             \x20       auto-merge\n\
             \x20       elements = { 203.0.113.7/32, 198.51.100.0/24 }\n\
             \x20   }\n\
             \x20   chain input {\n\
             \x20       type filter hook input priority -10; policy accept;\n\
             \x20       tcp dport 13306 ip saddr @allowed accept\n\
             \x20       tcp dport 13306 drop\n\
             \x20   }\n\
             }\n"
        );
        // Nothing allowed: the same, without elements (everything to the port is dropped).
        let closed = ruleset(13306, &[]);
        assert!(!closed.contains("elements"));
        assert!(closed.contains("tcp dport 13306 drop"));
    }

    #[test]
    fn bad_requests_are_refused_before_the_tool_is_run() {
        for bad in [
            vec!["0.0.0.0/0".to_owned()],
            vec!["x; flush ruleset".to_owned()],
        ] {
            let (code, _) = set(13306, &bad).unwrap_err();
            assert_eq!(code, ErrorCode::InvalidArguments);
        }
        let many: Vec<String> = (0..=MAX_CIDRS)
            .map(|i| format!("10.{}.{}.0/24", i / 256, i % 256))
            .collect();
        assert_eq!(
            set(13306, &many).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
    }
}
