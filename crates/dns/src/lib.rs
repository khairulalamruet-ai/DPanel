//! DNS providers and zone sync (T05.09).
//!
//! The panel keeps its own copy of every zone (`dns_zones`, `dns_records`); a
//! [`DnsProvider`] makes a DNS server match it. [`powerdns::PowerDns`] drives
//! PowerDNS Authoritative through its HTTP API; [`mock::MockProvider`] is an
//! in-memory server for tests of everything that uses a provider. A second
//! provider (Cloudflare) later only has to implement the trait.
//!
//! The provider works with **names without the final dot** for zones
//! (`example.com`) and the records of `builtpanel_domain::DnsRecord` (names relative
//! to the zone); the SOA and the apex name servers are the server's own and are
//! neither listed nor changed.

pub mod mock;
pub mod powerdns;
pub mod resolver;
pub mod sync;

use std::{future::Future, pin::Pin};

use builtpanel_domain::DnsRecord;

/// A boxed future, so providers can be used as `dyn DnsProvider`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a zone is created with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneSpec {
    /// The zone's name without the final dot.
    pub name: String,
    /// The zone's name servers, absolute (`ns1.example.net.`). May be empty
    /// until the name servers are configured.
    pub nameservers: Vec<String>,
}

/// Why a call to the DNS server failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DnsError {
    /// The server could not be reached or had a problem of its own (timeout,
    /// connection refused, 5xx, 429): trying again later may work.
    #[error("the DNS server is not available: {0}")]
    Unavailable(String),
    /// The server understood and refused (bad data, unknown zone, wrong key):
    /// trying the same again will fail the same way.
    #[error("the DNS server refused the request: {0}")]
    Rejected(String),
}

impl DnsError {
    /// Whether the same call may succeed later.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// A DNS server the panel can make match its zones.
pub trait DnsProvider: Send + Sync + 'static {
    /// A short name for logs (`powerdns`).
    fn name(&self) -> &'static str;

    /// Creates the zone if the server does not have it. Having it is success.
    fn ensure_zone<'a>(&'a self, zone: &'a ZoneSpec) -> BoxFuture<'a, Result<(), DnsError>>;

    /// Makes the zone hold exactly `records` (besides the server's own SOA and
    /// apex name servers): missing ones are added, changed ones replaced, others
    /// removed. The zone must exist.
    fn replace_records<'a>(
        &'a self,
        zone: &'a str,
        records: &'a [DnsRecord],
    ) -> BoxFuture<'a, Result<(), DnsError>>;

    /// The records the server has for the zone, or `None` if it has no such zone.
    fn list_records<'a>(
        &'a self,
        zone: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<DnsRecord>>, DnsError>>;

    /// Removes the zone. Not having it is success.
    fn delete_zone<'a>(&'a self, zone: &'a str) -> BoxFuture<'a, Result<(), DnsError>>;
}
