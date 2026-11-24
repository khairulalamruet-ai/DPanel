//! Built Panel's side of licensing (T17.07, T17.09): the installation's identity ([`identity`]),
//! the client of the license authority ([`client`]) and what a stored license allows
//! ([`state`]).

pub mod client;
pub mod identity;
pub mod state;

pub use client::{
    Beat, ClientError, DomainProof, LicenseClient, LicenseSummary, Received, ScanChallenge,
    ScanResult,
};
pub use identity::{Identity, IdentityError};
pub use state::{Evaluation, Limit, Mode, Policy, Stored, evaluate};

use builtpanel_license::{KeyError, PublicKeys};

/// The authority's public keys that ship with this build.
pub const BUNDLED_KEYS: &str = include_str!("../keys/license-public-keys.txt");

/// The keys to trust: the file at `path` when it is given, else the bundled ones.
///
/// # Errors
/// [`KeyError`] for an unreadable or malformed file.
pub fn trusted_keys(path: Option<&std::path::Path>) -> Result<PublicKeys, KeyError> {
    match path {
        Some(p) => {
            let text =
                std::fs::read_to_string(p).map_err(|e| KeyError::File(e.kind().to_string()))?;
            PublicKeys::parse(&text)
        }
        None => PublicKeys::parse(BUNDLED_KEYS),
    }
}
