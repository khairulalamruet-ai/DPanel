//! An in-memory DNS server for tests.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use builtpanel_domain::DnsRecord;

use crate::{BoxFuture, DnsError, DnsProvider, ZoneSpec};

/// What the mock holds and what was asked of it.
#[derive(Debug, Default)]
pub struct State {
    /// Zones by name, with their records.
    pub zones: BTreeMap<String, Vec<DnsRecord>>,
    /// Name servers each zone was created with.
    pub nameservers: BTreeMap<String, Vec<String>>,
    /// Every call, as `kind zone`, in order.
    pub calls: Vec<String>,
    /// Errors to hand out, one per call, oldest first (then calls succeed).
    pub failures: Vec<DnsError>,
}

/// An in-memory [`DnsProvider`]; clones share the same state.
#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    state: Arc<Mutex<State>>,
}

impl MockProvider {
    /// An empty server.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks at (and may change) the state.
    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        // A test that panicked while holding the state must not hide the next one.
        f(&mut self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// Makes the next call fail with `error`.
    pub fn fail_next(&self, error: DnsError) {
        self.with(|s| s.failures.push(error));
    }

    fn call(&self, kind: &str, zone: &str) -> Result<(), DnsError> {
        self.with(|s| {
            s.calls.push(format!("{kind} {zone}"));
            if s.failures.is_empty() {
                Ok(())
            } else {
                Err(s.failures.remove(0))
            }
        })
    }
}

impl DnsProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn ensure_zone<'a>(&'a self, zone: &'a ZoneSpec) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            self.call("ensure_zone", &zone.name)?;
            self.with(|s| {
                s.zones.entry(zone.name.clone()).or_default();
                s.nameservers
                    .entry(zone.name.clone())
                    .or_insert_with(|| zone.nameservers.clone());
            });
            Ok(())
        })
    }

    fn replace_records<'a>(
        &'a self,
        zone: &'a str,
        records: &'a [DnsRecord],
    ) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            self.call("replace_records", zone)?;
            self.with(|s| match s.zones.get_mut(zone) {
                Some(held) => {
                    held.clear();
                    held.extend_from_slice(records);
                    Ok(())
                }
                None => Err(DnsError::Rejected(format!("no zone {zone}"))),
            })
        })
    }

    fn list_records<'a>(
        &'a self,
        zone: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<DnsRecord>>, DnsError>> {
        Box::pin(async move {
            self.call("list_records", zone)?;
            Ok(self.with(|s| s.zones.get(zone).cloned()))
        })
    }

    fn delete_zone<'a>(&'a self, zone: &'a str) -> BoxFuture<'a, Result<(), DnsError>> {
        Box::pin(async move {
            self.call("delete_zone", zone)?;
            self.with(|s| {
                s.zones.remove(zone);
                s.nameservers.remove(zone);
            });
            Ok(())
        })
    }
}

/// What a name will answer.
type Answers = BTreeMap<String, Result<Vec<String>, DnsError>>;

/// A resolver that answers from a table, for tests.
#[derive(Debug, Clone, Default)]
pub struct MockResolver {
    answers: Arc<Mutex<Answers>>,
}

impl MockResolver {
    /// A resolver that knows nothing (every name has no name servers).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What `name` (without the final dot) will answer.
    pub fn answer(&self, name: &str, answer: Result<Vec<String>, DnsError>) {
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.to_owned(), answer);
    }
}

impl crate::resolver::NsResolver for MockResolver {
    fn lookup_ns<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DnsError>> {
        Box::pin(async move {
            self.answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(name.trim_end_matches('.'))
                .cloned()
                .unwrap_or(Ok(Vec::new()))
        })
    }
}
