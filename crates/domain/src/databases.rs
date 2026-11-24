//! Names and access levels of databases and database users (T07.02).
//!
//! A database or user belongs to one account, and its name starts with the
//! account's system name and an underscore (`alice_shop`), so two accounts never
//! meet in a name and a person can tell whose a database is. The prefix can be
//! switched off by the administrator (a panel that migrates names from elsewhere
//! needs that); every other rule stays: lower-case letters, digits and `_`, at
//! most 64 characters, and nothing that belongs to the database server itself.

use crate::TenantUsername;

/// Longest database or user name.
pub const MAX_NAME_LEN: usize = 64;

/// Names that belong to the database server (or that people would take for them).
const RESERVED: &[&str] = &[
    "mysql",
    "information_schema",
    "performance_schema",
    "sys",
    "test",
    "root",
    "mariadb",
    "mariadb_sys",
    "admin",
    "administrator",
    "builtpanel",
    "pma",
    "phpmyadmin",
    "debian_sys_maint",
    "healthcheck",
];

/// Why a name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DbNameError {
    /// Empty, or something other than `a-z`, `0-9` and `_`.
    #[error("a name is lower-case letters, digits and underscores")]
    Invalid,
    /// Longer than [`MAX_NAME_LEN`] with the prefix.
    #[error("the whole name, with the account prefix, is at most 64 characters")]
    TooLong,
    /// Belongs to the database server.
    #[error("this name is reserved")]
    Reserved,
}

impl DbNameError {
    /// Stable machine code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "invalid_db_name",
            Self::TooLong => "db_name_too_long",
            Self::Reserved => "reserved_db_name",
        }
    }
}

fn plain(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The whole name of a database or user from what a person typed: with the
/// account's prefix (`alice_` + `shop`) when `with_prefix`, as typed otherwise.
///
/// A person may type the name with the prefix already (`alice_shop`): it is not
/// put on twice.
///
/// # Errors
/// [`DbNameError`].
pub fn full_name(
    tenant: &TenantUsername,
    typed: &str,
    with_prefix: bool,
) -> Result<String, DbNameError> {
    let typed = typed.trim().to_ascii_lowercase();
    let prefix = format!("{}_", tenant.as_str());
    let name = if with_prefix && !typed.starts_with(&prefix) {
        format!("{prefix}{typed}")
    } else {
        typed.clone()
    };
    // The part after the prefix has to be there and plain.
    let own = name.strip_prefix(&prefix).unwrap_or(&name);
    if !plain(&name) || !plain(own) {
        return Err(DbNameError::Invalid);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(DbNameError::TooLong);
    }
    if RESERVED.contains(&name.as_str()) || RESERVED.contains(&own) {
        return Err(DbNameError::Reserved);
    }
    Ok(name)
}

/// Checks a database or user name as the node is about to use it in a statement:
/// plain characters, within the length, and nothing that belongs to the server.
/// (The account prefix is the control plane's business; the node only has to be
/// sure the name cannot be anything but an identifier.)
///
/// # Errors
/// [`DbNameError`].
pub fn check_identifier(name: &str) -> Result<(), DbNameError> {
    if !plain(name) {
        return Err(DbNameError::Invalid);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(DbNameError::TooLong);
    }
    if RESERVED.contains(&name) {
        return Err(DbNameError::Reserved);
    }
    Ok(())
}

/// Shortest and longest password of a database user.
pub const PASSWORD_LEN: std::ops::RangeInclusive<usize> = 12..=128;

/// Why a password was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    /// Shorter than 12 or longer than 128 characters.
    #[error("a password is 12 to 128 characters")]
    Length,
    /// Not printable ASCII.
    #[error("a password is made of printable ASCII characters")]
    Characters,
    /// Fewer than three of: lower case, upper case, digits, symbols.
    #[error("a password mixes at least three of: lower case, upper case, digits, symbols")]
    Weak,
}

impl PasswordError {
    /// Stable machine code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Length => "password_length",
            Self::Characters => "password_characters",
            Self::Weak => "password_weak",
        }
    }
}

/// Checks the password of a database user.
///
/// # Errors
/// [`PasswordError`].
pub fn check_password(password: &str) -> Result<(), PasswordError> {
    if !PASSWORD_LEN.contains(&password.chars().count()) {
        return Err(PasswordError::Length);
    }
    if !password.chars().all(|c| (' '..='~').contains(&c)) {
        return Err(PasswordError::Characters);
    }
    let classes = [
        password.chars().any(|c| c.is_ascii_lowercase()),
        password.chars().any(|c| c.is_ascii_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if classes < 3 {
        return Err(PasswordError::Weak);
    }
    Ok(())
}

/// The host part of a database user for connections from one network (an IPv4
/// range like `172.18.0.0/16`), as the server writes it: `172.18.0.0/255.255.0.0`.
///
/// # Errors
/// `None` for anything but an IPv4 range with a prefix of 8 to 30.
#[must_use]
pub fn host_for_network(cidr: &str) -> Option<String> {
    let (ip, bits) = cidr.split_once('/')?;
    let bits: u32 = bits.parse().ok().filter(|b| (8..=30).contains(b))?;
    let parts: Vec<u8> = ip
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 4 || ip.split('.').any(|p| p.len() > 1 && p.starts_with('0')) {
        return None;
    }
    let mask = u32::MAX << (32 - bits);
    let m = mask.to_be_bytes();
    // The address must be the start of the range (host bits zero).
    let addr = u32::from_be_bytes([parts[0], parts[1], parts[2], parts[3]]);
    if addr & !mask != 0 {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}/{}.{}.{}.{}",
        parts[0], parts[1], parts[2], parts[3], m[0], m[1], m[2], m[3]
    ))
}

/// Most addresses a user may be allowed to connect from besides its own network.
pub const MAX_REMOTE_HOSTS: usize = 20;

/// The host part of a database user for connections from a remote address or
/// range given as IPv4 CIDR (`203.0.113.7/32`, `198.51.100.0/24`), as the server
/// writes it. Wider than a /8, or anything that is not the start of its range, is
/// refused: a range is never "everywhere".
#[must_use]
pub fn host_for_remote(cidr: &str) -> Option<String> {
    let (ip, bits) = cidr.split_once('/')?;
    let bits: u32 = bits.parse().ok().filter(|b| (8..=32).contains(b))?;
    if bits == 32 {
        let parts: Vec<u8> = ip
            .split('.')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if parts.len() != 4 || ip.split('.').any(|p| p.len() > 1 && p.starts_with('0')) {
            return None;
        }
        return Some(format!("{ip}/255.255.255.255"));
    }
    host_for_network(cidr)
}

/// What a user may do in a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Read only.
    ReadOnly,
    /// Read and change data.
    ReadWrite,
    /// Everything inside that database (tables, indexes, routines, ...).
    All,
}

impl Access {
    /// The name used in the API and the ledger.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
            Self::All => "all",
        }
    }

    /// Reads a name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::ReadOnly, Self::ReadWrite, Self::All]
            .into_iter()
            .find(|a| a.as_str() == text)
    }

    /// The privileges it stands for, as the server names them. A fixed list: no
    /// request ever supplies one, and none of them is `GRANT OPTION`, `FILE`,
    /// `SUPER` or anything outside the one database.
    #[must_use]
    pub fn privileges(self) -> &'static [&'static str] {
        match self {
            Self::ReadOnly => &["SELECT", "SHOW VIEW"],
            Self::ReadWrite => &["SELECT", "INSERT", "UPDATE", "DELETE", "SHOW VIEW"],
            Self::All => &[
                "SELECT",
                "INSERT",
                "UPDATE",
                "DELETE",
                "CREATE",
                "DROP",
                "ALTER",
                "INDEX",
                "REFERENCES",
                "CREATE TEMPORARY TABLES",
                "LOCK TABLES",
                "CREATE VIEW",
                "SHOW VIEW",
                "CREATE ROUTINE",
                "ALTER ROUTINE",
                "EXECUTE",
                "TRIGGER",
                "EVENT",
            ],
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn alice() -> TenantUsername {
        TenantUsername::parse("alice").unwrap()
    }

    #[test]
    fn names_get_the_accounts_prefix_once() {
        let a = alice();
        assert_eq!(full_name(&a, "shop", true).unwrap(), "alice_shop");
        assert_eq!(full_name(&a, "alice_shop", true).unwrap(), "alice_shop");
        assert_eq!(full_name(&a, "  Shop_2 ", true).unwrap(), "alice_shop_2");
        // Without the prefix rule the name is as typed (still checked).
        assert_eq!(full_name(&a, "shop", false).unwrap(), "shop");
        assert_eq!(full_name(&a, "alice_shop", false).unwrap(), "alice_shop");
    }

    #[test]
    fn what_cannot_be_an_identifier_or_belongs_to_the_server_is_refused() {
        let a = alice();
        for (typed, err) in [
            ("", DbNameError::Invalid),
            ("   ", DbNameError::Invalid),
            ("sh op", DbNameError::Invalid),
            ("shop-2", DbNameError::Invalid),
            ("shop;drop", DbNameError::Invalid),
            ("a`b", DbNameError::Invalid),
            ("sh'op", DbNameError::Invalid),
            ("sh\nop", DbNameError::Invalid),
            ("ünï", DbNameError::Invalid),
            ("alice_", DbNameError::Invalid),
            ("mysql", DbNameError::Reserved),
            ("information_schema", DbNameError::Reserved),
            ("alice_mysql", DbNameError::Reserved),
            ("root", DbNameError::Reserved),
        ] {
            assert_eq!(full_name(&a, typed, true).unwrap_err(), err, "{typed:?}");
        }
        // Without the prefix a server name is still reserved.
        for typed in ["mysql", "sys", "performance_schema", "root"] {
            assert_eq!(
                full_name(&a, typed, false).unwrap_err(),
                DbNameError::Reserved
            );
        }
        // The whole name, prefix included, has a limit.
        let long = "x".repeat(MAX_NAME_LEN - "alice_".len());
        assert!(full_name(&a, &long, true).is_ok());
        assert_eq!(
            full_name(&a, &format!("{long}x"), true).unwrap_err(),
            DbNameError::TooLong
        );
        assert_eq!(DbNameError::Reserved.code(), "reserved_db_name");
    }

    #[test]
    fn the_node_accepts_only_plain_identifiers() {
        for ok in ["alice_shop", "a", "x_1", &"a".repeat(MAX_NAME_LEN)] {
            assert!(check_identifier(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "a;DROP DATABASE mysql",
            "a`b",
            "a b",
            "a-b",
            "A",
            "a'b",
            "a\\b",
            "a\0b",
            "mysql",
            "root",
            &"a".repeat(MAX_NAME_LEN + 1),
        ] {
            assert!(check_identifier(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn passwords_are_long_printable_and_mixed() {
        for ok in [
            "Abcdef123456",
            "correct-Horse-9",
            "aaaaaaaaaa1A",
            &format!("Aa1{}", "x".repeat(125)),
        ] {
            assert!(check_password(ok).is_ok(), "{ok}");
        }
        for (bad, err) in [
            ("", PasswordError::Length),
            ("Short1!", PasswordError::Length),
            (&format!("Aa1{}", "x".repeat(126)), PasswordError::Length),
            ("Abcdef12345\n", PasswordError::Characters),
            ("Abcdef12345\u{e9}", PasswordError::Characters),
            ("alllowercaseonly", PasswordError::Weak),
            (
                "ALLUPPERCASE1234"[..12].to_lowercase().as_str(),
                PasswordError::Weak,
            ),
            ("abcdefghijkl12", PasswordError::Weak),
        ] {
            assert_eq!(check_password(bad).unwrap_err(), err, "{bad:?}");
        }
        assert_eq!(PasswordError::Weak.code(), "password_weak");
    }

    #[test]
    fn a_network_becomes_the_host_part_of_a_user() {
        assert_eq!(
            host_for_network("172.18.0.0/16").as_deref(),
            Some("172.18.0.0/255.255.0.0")
        );
        assert_eq!(
            host_for_network("10.1.2.0/24").as_deref(),
            Some("10.1.2.0/255.255.255.0")
        );
        assert_eq!(
            host_for_network("192.168.0.0/20").as_deref(),
            Some("192.168.0.0/255.255.240.0")
        );
        for bad in [
            "172.18.0.1/16",
            "172.18.0.0/7",
            "172.18.0.0/31",
            "172.18.0.0",
            "fd00::/64",
            "172.18.0/16",
            "172.18.0.0.0/16",
            "172.18.0.256/16",
            "172.018.0.0/16",
            "%",
            "172.18.0.0/16'; DROP",
        ] {
            assert_eq!(host_for_network(bad), None, "{bad}");
        }
    }

    #[test]
    fn remote_addresses_are_single_hosts_or_ranges_never_everywhere() {
        assert_eq!(
            host_for_remote("203.0.113.7/32").as_deref(),
            Some("203.0.113.7/255.255.255.255")
        );
        assert_eq!(
            host_for_remote("198.51.100.0/24").as_deref(),
            Some("198.51.100.0/255.255.255.0")
        );
        for bad in [
            "0.0.0.0/0",
            "0.0.0.0/1",
            "10.0.0.0/7",
            "203.0.113.7",
            "203.0.113.7/33",
            "203.0.113.7/24",
            "203.0.113.007/32",
            "::1/128",
            "%",
            "203.0.113.7/32; --",
            "",
        ] {
            assert_eq!(host_for_remote(bad), None, "{bad}");
        }
    }

    #[test]
    fn access_levels_name_fixed_privileges() {
        for a in [Access::ReadOnly, Access::ReadWrite, Access::All] {
            assert_eq!(Access::parse(a.as_str()), Some(a));
            for p in a.privileges() {
                assert!(
                    p.bytes().all(|b| b.is_ascii_uppercase() || b == b' '),
                    "{p}"
                );
                for never in ["GRANT", "FILE", "SUPER", "PROCESS", "SHUTDOWN", "RELOAD"] {
                    assert!(!p.contains(never), "{p}");
                }
            }
        }
        assert_eq!(Access::parse("root"), None);
        assert!(Access::ReadOnly.privileges().len() < Access::ReadWrite.privileges().len());
        assert!(Access::ReadWrite.privileges().len() < Access::All.privileges().len());
    }
}
