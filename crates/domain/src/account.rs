//! Account value objects.
//!
//! The rules here mirror the CHECK constraints in
//! `migrations/20261111000002_accounts_sessions_login_events.up.sql`; keep both
//! in sync so input is rejected with a clear message before reaching the DB.

use std::fmt;

/// Kind of account. Mirrors the `account_kind` PostgreSQL enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountKind {
    /// Panel operator.
    Admin,
    /// Sells hosting to its own clients (v2).
    Reseller,
    /// Hosting customer.
    Client,
}

impl AccountKind {
    /// Database / wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Reseller => "reseller",
            Self::Client => "client",
        }
    }
}

/// Why a value was rejected. `code()` is stable for API clients and i18n.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// Username does not match `^[a-z][a-z0-9_-]{2,31}$` after normalisation.
    #[error(
        "username must be 3-32 characters, start with a letter and use only a-z, 0-9, '_' or '-'"
    )]
    InvalidUsername,
    /// Username is reserved for the system.
    #[error("username is reserved")]
    ReservedUsername,
    /// Email is not a plausible address.
    #[error("email address is not valid")]
    InvalidEmail,
}

impl ValidationError {
    /// Stable machine code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUsername => "invalid_username",
            Self::ReservedUsername => "reserved_username",
            Self::InvalidEmail => "invalid_email",
        }
    }
}

/// Names that would collide with system users, services or URL paths.
pub(crate) const RESERVED_USERNAMES: &[&str] = &[
    "root",
    "admin",
    "administrator",
    "builtpanel",
    "system",
    "daemon",
    "nobody",
    "postgres",
    "redis",
    "mysql",
    "www",
    "www-data",
    "mail",
    "postmaster",
    "hostmaster",
    "webmaster",
    "abuse",
    "support",
    "security",
    "api",
    "ftp",
    "ssh",
    "sudo",
    "bin",
    "sys",
    "sync",
    "operator",
    "caddy",
    "nginx",
    "apache",
    "docker",
    "test",
    "guest",
    "info",
    "noreply",
];

/// A validated, lowercase username.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    /// Trims, lowercases and validates.
    ///
    /// # Errors
    /// [`ValidationError::InvalidUsername`] or [`ValidationError::ReservedUsername`].
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        let s = input.trim().to_ascii_lowercase();
        let mut chars = s.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok =
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !(first_ok && rest_ok && (3..=32).contains(&s.len())) {
            return Err(ValidationError::InvalidUsername);
        }
        if RESERVED_USERNAMES.contains(&s.as_str()) {
            return Err(ValidationError::ReservedUsername);
        }
        Ok(Self(s))
    }

    /// The normalised value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated, lowercase email address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    /// Trims, lowercases and checks the shape `local@domain.tld` (no spaces,
    /// exactly one `@`, a dot in the domain, at most 254 bytes). Deliverability
    /// is verified separately (confirmation email), not here.
    ///
    /// # Errors
    /// [`ValidationError::InvalidEmail`].
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        let s = input.trim().to_lowercase();
        let valid = s.len() <= 254
            && !s.chars().any(char::is_whitespace)
            && s.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain
                        .rsplit_once('.')
                        .is_some_and(|(host, tld)| !host.is_empty() && !tld.is_empty())
            });
        if valid {
            Ok(Self(s))
        } else {
            Err(ValidationError::InvalidEmail)
        }
    }

    /// The normalised value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_normalised_and_validated() {
        assert_eq!(Username::parse("  Khairul ").unwrap().as_str(), "khairul");
        for ok in ["abc", "a_b-c9", "z".repeat(32).as_str()] {
            assert!(Username::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "ab",
            "1abc",
            "_abc",
            "has space",
            "ünï",
            "a".repeat(33).as_str(),
            "",
        ] {
            assert_eq!(
                Username::parse(bad),
                Err(ValidationError::InvalidUsername),
                "{bad}"
            );
        }
    }

    #[test]
    fn reserved_usernames_are_refused_case_insensitively() {
        for r in ["root", "Admin", "POSTGRES", "www-data"] {
            assert_eq!(
                Username::parse(r),
                Err(ValidationError::ReservedUsername),
                "{r}"
            );
        }
    }

    #[test]
    fn emails_are_normalised_and_validated() {
        assert_eq!(
            Email::parse(" Khairul@Example.COM ").unwrap().as_str(),
            "khairul@example.com"
        );
        for ok in ["a@b.co", "first.last+tag@sub.example.org"] {
            assert!(Email::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "no-at",
            "a@b",
            "@example.com",
            "a@.com",
            "a@b.",
            "a b@example.com",
            "a@b@c.com",
        ] {
            assert_eq!(
                Email::parse(bad),
                Err(ValidationError::InvalidEmail),
                "{bad}"
            );
        }
        let long = format!("{}@example.com", "a".repeat(250));
        assert_eq!(Email::parse(&long), Err(ValidationError::InvalidEmail));
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(ValidationError::InvalidUsername.code(), "invalid_username");
        assert_eq!(ValidationError::InvalidEmail.code(), "invalid_email");
        assert_eq!(AccountKind::Admin.as_str(), "admin");
    }
}
