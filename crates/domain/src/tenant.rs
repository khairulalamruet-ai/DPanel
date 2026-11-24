//! Tenants: one hosting account's footprint on a node (T04.05).
//!
//! A tenant is a system user with its own numeric id, a directory under
//! `/srv/tenants/<id>`, and a container stack. The rules here decide what a
//! tenant may be called and which numeric id it gets, and mirror the CHECK
//! constraints of `migrations/20261111000016_tenants.up.sql`.

use std::fmt;

use uuid::Uuid;

use crate::account::RESERVED_USERNAMES;

/// Where tenant data lives on a node.
pub const TENANT_ROOT: &str = "/srv/tenants";

/// Why a tenant value was rejected. `code()` is stable for API clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TenantError {
    /// Not 3-32 characters of `a-z` and `0-9`, starting with a letter.
    #[error("the system name must be 3-32 characters of a-z and 0-9, starting with a letter")]
    InvalidUsername,
    /// Reserved for the system (`root`, `www`, ...) or looks like a service account.
    #[error("this system name is reserved")]
    ReservedUsername,
    /// The node has no free numeric id left.
    #[error("the node has no free user ids left")]
    NoFreeUid,
    /// Not one of the known statuses.
    #[error("unknown tenant status")]
    UnknownStatus,
    /// The status change is not allowed.
    #[error("a tenant cannot go from {from} to {to}")]
    BadTransition {
        /// Where it is.
        from: TenantStatus,
        /// Where it was asked to go.
        to: TenantStatus,
    },
}

impl TenantError {
    /// Stable machine code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUsername => "invalid_tenant_username",
            Self::ReservedUsername => "reserved_tenant_username",
            Self::NoFreeUid => "no_free_uid",
            Self::UnknownStatus => "unknown_tenant_status",
            Self::BadTransition { .. } => "bad_tenant_transition",
        }
    }
}

/// The system name of a tenant: lower-case letters and digits only, so it is
/// safe as a user name, a directory name and a container label everywhere.
/// Stricter than a panel login name (no `_` or `-`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantUsername(String);

impl TenantUsername {
    /// Trims, lower-cases and validates.
    ///
    /// # Errors
    /// [`TenantError::InvalidUsername`] or [`TenantError::ReservedUsername`].
    pub fn parse(input: &str) -> Result<Self, TenantError> {
        let s = input.trim().to_ascii_lowercase();
        let mut chars = s.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !(first_ok && rest_ok && (3..=32).contains(&s.len())) {
            return Err(TenantError::InvalidUsername);
        }
        // Reserved names, and the numbered families the system itself uses
        // (`docker1`, `user1000`, `dh7`). A name that is only digits is already
        // refused above: it has to start with a letter.
        let numbered = |prefix: &str| {
            s.strip_prefix(prefix)
                .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
        };
        let reserved = RESERVED_USERNAMES.contains(&s.as_str())
            || ["docker", "systemd", "builtpanel", "bp", "user"]
                .iter()
                .any(|p| numbered(p));
        if reserved {
            return Err(TenantError::ReservedUsername);
        }
        Ok(Self(s))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantUsername {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A numeric user (and group) id inside the agent's user namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(u32);

impl Uid {
    /// The number.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The ids a node hands out to tenants: `start..end` (end excluded).
///
/// Containers run in the agent's rootless user namespace, where ids 0..65535
/// exist. Tenants take 10000..60000: below are the system's own and the
/// image's, above is left free (`nobody` is 65534). A node can therefore host
/// 50000 tenants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UidRange {
    start: u32,
    end: u32,
}

impl UidRange {
    /// The range every node uses.
    pub const DEFAULT: Self = Self {
        start: 10_000,
        end: 60_000,
    };

    /// A range; `None` when empty or reaching into the system ids below 1000.
    #[must_use]
    pub fn new(start: u32, end: u32) -> Option<Self> {
        (start >= 1000 && start < end && end <= 65_535).then_some(Self { start, end })
    }

    /// First id.
    #[must_use]
    pub fn start(self) -> u32 {
        self.start
    }

    /// One past the last id.
    #[must_use]
    pub fn end(self) -> u32 {
        self.end
    }

    /// How many ids there are.
    #[must_use]
    pub fn capacity(self) -> u32 {
        self.end - self.start
    }

    /// Whether `uid` is in the range.
    #[must_use]
    pub fn contains(self, uid: u32) -> bool {
        (self.start..self.end).contains(&uid)
    }

    /// `uid` as a [`Uid`], if it lies inside the range.
    #[must_use]
    pub fn checked(self, uid: u32) -> Option<Uid> {
        self.contains(uid).then_some(Uid(uid))
    }

    /// The lowest id in the range that is not in `used` (which may be in any
    /// order, hold duplicates and ids outside the range).
    ///
    /// # Errors
    /// [`TenantError::NoFreeUid`] when every id is taken.
    pub fn allocate(self, used: impl IntoIterator<Item = u32>) -> Result<Uid, TenantError> {
        let mut taken: Vec<u32> = used.into_iter().filter(|u| self.contains(*u)).collect();
        taken.sort_unstable();
        taken.dedup();
        let mut candidate = self.start;
        for uid in taken {
            if uid == candidate {
                candidate += 1;
            } else {
                break; // a gap: `candidate` is free
            }
        }
        if candidate < self.end {
            Ok(Uid(candidate))
        } else {
            Err(TenantError::NoFreeUid)
        }
    }
}

/// Where a tenant is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TenantStatus {
    /// Being set up; not usable yet.
    Provisioning,
    /// Running.
    Active,
    /// Stopped by an administrator; the data stays.
    Suspended,
    /// Deleted, waiting for its data to be purged.
    Deleting,
    /// Setting up failed; an administrator decides to retry or delete.
    Failed,
}

impl TenantStatus {
    /// Database / wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleting => "deleting",
            Self::Failed => "failed",
        }
    }

    /// Reads the database / wire representation.
    ///
    /// # Errors
    /// [`TenantError::UnknownStatus`].
    pub fn parse(s: &str) -> Result<Self, TenantError> {
        match s {
            "provisioning" => Ok(Self::Provisioning),
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "deleting" => Ok(Self::Deleting),
            "failed" => Ok(Self::Failed),
            _ => Err(TenantError::UnknownStatus),
        }
    }

    /// Whether a tenant may go from this status to `to`.
    ///
    /// ```text
    /// provisioning -> active | failed | deleting
    /// failed       -> provisioning (retry) | deleting
    /// active       -> suspended | deleting
    /// suspended    -> active | deleting
    /// deleting     -> (nothing: the purge removes the row)
    /// ```
    #[must_use]
    pub fn can_become(self, to: Self) -> bool {
        use TenantStatus::{Active, Deleting, Failed, Provisioning, Suspended};
        matches!(
            (self, to),
            (Provisioning, Active | Failed | Deleting)
                | (Failed, Provisioning | Deleting)
                | (Active, Suspended | Deleting)
                | (Suspended, Active | Deleting)
        )
    }

    /// Checks a change.
    ///
    /// # Errors
    /// [`TenantError::BadTransition`].
    pub fn transition(self, to: Self) -> Result<Self, TenantError> {
        if self.can_become(to) {
            Ok(to)
        } else {
            Err(TenantError::BadTransition { from: self, to })
        }
    }
}

impl fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tenant as the rest of the system sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    /// Id (also the directory name).
    pub id: Uuid,
    /// System user name.
    pub username: TenantUsername,
    /// Numeric user and group id.
    pub uid: Uid,
    /// Where it is in its life.
    pub status: TenantStatus,
}

impl Tenant {
    /// The tenant's directory on its node: `/srv/tenants/<id>`.
    #[must_use]
    pub fn root(&self) -> String {
        format!("{TENANT_ROOT}/{}", self.id)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn system_names_are_plain_and_not_reserved() {
        for ok in [
            "alice",
            "shop2024",
            "abc",
            "a1b2c3",
            &format!("a{}", "b".repeat(31)),
        ] {
            assert!(TenantUsername::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            TenantUsername::parse("  Alice ").unwrap().as_str(),
            "alice",
            "trimmed and lower-cased"
        );
        for bad in [
            "",
            "ab",
            "1abc",
            "has space",
            "under_score",
            "dash-ed",
            "ünï",
            "a.b",
            &format!("a{}", "b".repeat(32)),
        ] {
            assert_eq!(
                TenantUsername::parse(bad),
                Err(TenantError::InvalidUsername),
                "{bad:?}"
            );
        }
        for reserved in [
            "root",
            "www",
            "mysql",
            "postgres",
            "nobody",
            "docker",
            "docker1",
            "builtpanel2",
            "systemd",
            "user",
            "user1000",
            "mail",
            "ftp",
        ] {
            assert_eq!(
                TenantUsername::parse(reserved),
                Err(TenantError::ReservedUsername),
                "{reserved}"
            );
        }
        // Ordinary words that merely start like a reserved family are fine.
        for ok in ["userland", "docking", "shopdh", "dhaka", "dhall", "users"] {
            assert!(TenantUsername::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            TenantError::InvalidUsername.code(),
            "invalid_tenant_username"
        );
    }

    #[test]
    fn ids_are_handed_out_lowest_free_first() {
        let range = UidRange::DEFAULT;
        assert_eq!(range.allocate([]).unwrap().get(), 10_000);
        assert_eq!(
            range.allocate([10_000, 10_001, 10_002]).unwrap().get(),
            10_003
        );
        // A gap is reused; order, duplicates and outside ids do not matter.
        assert_eq!(
            range
                .allocate([10_002, 10_000, 10_000, 5, 70_000])
                .unwrap()
                .get(),
            10_001
        );
        assert_eq!(
            range.allocate([9_999, 60_000, 65_534]).unwrap().get(),
            10_000
        );
        assert_eq!(range.capacity(), 50_000);
    }

    #[test]
    fn a_full_node_has_no_free_id() {
        let small = UidRange::new(1000, 1003).unwrap();
        assert_eq!(small.allocate([1000, 1001]).unwrap().get(), 1002);
        assert_eq!(
            small.allocate([1000, 1001, 1002]),
            Err(TenantError::NoFreeUid)
        );
        assert_eq!(TenantError::NoFreeUid.code(), "no_free_uid");
        // A range may not reach into system ids, be empty, or pass the namespace.
        assert!(UidRange::new(999, 2000).is_none());
        assert!(UidRange::new(2000, 2000).is_none());
        assert!(UidRange::new(2000, 65_536).is_none());
        assert!(UidRange::DEFAULT.contains(10_000) && !UidRange::DEFAULT.contains(60_000));
    }

    #[test]
    fn a_tenant_moves_only_along_its_life_cycle() {
        use TenantStatus::*;
        let all = [Provisioning, Active, Suspended, Deleting, Failed];
        let allowed: Vec<(TenantStatus, TenantStatus)> = all
            .iter()
            .flat_map(|from| all.iter().map(move |to| (*from, *to)))
            .filter(|(from, to)| from.can_become(*to))
            .collect();
        assert_eq!(
            allowed,
            [
                (Provisioning, Active),
                (Provisioning, Deleting),
                (Provisioning, Failed),
                (Active, Suspended),
                (Active, Deleting),
                (Suspended, Active),
                (Suspended, Deleting),
                (Failed, Provisioning),
                (Failed, Deleting),
            ]
        );
        assert_eq!(Active.transition(Suspended), Ok(Suspended));
        assert_eq!(
            Deleting.transition(Active),
            Err(TenantError::BadTransition {
                from: Deleting,
                to: Active
            })
        );
        assert!(!Active.can_become(Active), "no-op changes are not changes");
        for s in all {
            assert_eq!(TenantStatus::parse(s.as_str()), Ok(s));
        }
        assert_eq!(TenantStatus::parse("gone"), Err(TenantError::UnknownStatus));
    }

    #[test]
    fn a_tenant_lives_in_its_own_directory() {
        let id: Uuid = "0190b6a2-0000-7000-8000-000000000001".parse().unwrap();
        let tenant = Tenant {
            id,
            username: TenantUsername::parse("alice").unwrap(),
            uid: UidRange::DEFAULT.allocate([]).unwrap(),
            status: TenantStatus::Provisioning,
        };
        assert_eq!(
            tenant.root(),
            "/srv/tenants/0190b6a2-0000-7000-8000-000000000001"
        );
    }
}
