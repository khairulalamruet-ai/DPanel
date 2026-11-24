//! Proof that an installation is really served at the domain it claims.
//!
//! The binding (see [`crate::binding`]) already checks the address a request comes from. A
//! domain, though, is only *stated* by the installation. When this check is switched on, the
//! first activation of an installation is also tested from the outside: the authority fetches
//! `https://<domain>/api/v1/public/license-proof?nonce=...` and the answer must carry a signature
//! of that nonce and domain made with the installation's own key (the key it is registering).
//! Whoever answers there holds the key and answers for that name, so a license cannot be activated
//! for a domain that leads somewhere else.
//!
//! The authority connects to a name an installation gave, so it is careful: `https` on the
//! standard port only, no redirect is followed, a short time limit, a small answer, and the
//! name may only resolve to the addresses the license is bound to (an address chosen by the
//! operator when the license was issued): the connection is made to one of those, never to
//! whatever the name says.

use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use builtpanel_http_kit::AppError;
use serde::Deserialize;
use ureq::{
    Error as UreqError,
    config::Config,
    http::Uri,
    unversioned::{
        resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver},
        transport::{DefaultConnector, NextTimeout},
    },
};
use uuid::Uuid;

use crate::{LaState, public};

/// How long the panel has to answer.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The most bytes read from the answer.
const MAX_ANSWER: u64 = 4096;

/// A boxed future (so that the prover can be a trait object).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the panel answers.
#[derive(Debug, Clone, Deserialize)]
pub struct Answer {
    /// The installation that answers.
    pub installation_id: Uuid,
    /// The domain it signed for.
    pub domain: String,
    /// The signature, base64url.
    pub signature: String,
}

/// Fetches the panel's answer from the domain.
pub trait DomainProver: Send + Sync {
    /// Asks `domain` for its proof of `nonce`; the connection may only go to one of `allowed`.
    fn fetch<'a>(
        &'a self,
        domain: &'a str,
        nonce: &'a str,
        allowed: &'a [IpAddr],
    ) -> BoxFuture<'a, Result<Answer, String>>;
}

/// Answers only with the allowed addresses among those a name resolves to.
#[derive(Debug)]
struct AllowedOnly {
    inner: DefaultResolver,
    allowed: Vec<IpAddr>,
}

impl Resolver for AllowedOnly {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, UreqError> {
        let found = self.inner.resolve(uri, config, timeout)?;
        let mut kept = self.inner.empty();
        for address in found
            .iter()
            .filter(|a| self.allowed.contains(&a.ip().to_canonical()))
        {
            kept.push(*address);
        }
        if kept.is_empty() {
            return Err(UreqError::HostNotFound);
        }
        Ok(kept)
    }
}

/// The real thing: an HTTPS request with certificate checking.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpsProver;

impl DomainProver for HttpsProver {
    fn fetch<'a>(
        &'a self,
        domain: &'a str,
        nonce: &'a str,
        allowed: &'a [IpAddr],
    ) -> BoxFuture<'a, Result<Answer, String>> {
        let url = format!("https://{domain}/api/v1/public/license-proof?nonce={nonce}");
        let allowed = allowed.to_vec();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let config = Config::builder()
                    .timeout_global(Some(TIMEOUT))
                    .max_redirects(0)
                    .https_only(true)
                    .http_status_as_error(false)
                    .build();
                let agent = ureq::Agent::with_parts(
                    config,
                    DefaultConnector::default(),
                    AllowedOnly {
                        inner: DefaultResolver::default(),
                        allowed,
                    },
                );
                let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
                if response.status().as_u16() != 200 {
                    return Err(format!("the panel answered {}", response.status().as_u16()));
                }
                let body = response
                    .body_mut()
                    .with_config()
                    .limit(MAX_ANSWER)
                    .read_to_string()
                    .map_err(|e| e.to_string())?;
                serde_json::from_str::<Answer>(&body).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }
}

/// The prover a configuration asks for.
#[must_use]
pub fn default_prover() -> Arc<dyn DomainProver> {
    Arc::new(HttpsProver)
}

/// A fresh challenge: 24 random bytes, base64url (32 characters).
fn nonce() -> Result<String, AppError> {
    use std::io::Read as _;
    let mut raw = [0u8; 24];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut raw))
        .map_err(AppError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn refused(message: &str) -> AppError {
    AppError::forbidden("domain_proof_failed", message.to_owned())
}

/// Checks that `domain` answers for the installation that owns `public_key`.
///
/// # Errors
/// 403 `domain_proof_failed` when the domain cannot be reached, answers wrongly, or the answer is
/// not signed by the installation's key.
pub async fn verify(
    state: &LaState,
    installation_id: Uuid,
    public_key: &[u8],
    domain: &str,
    bound_ips: &[String],
) -> Result<(), AppError> {
    let allowed: Vec<IpAddr> = bound_ips
        .iter()
        .filter_map(|i| i.parse::<IpAddr>().ok())
        .map(|i| i.to_canonical())
        .collect();
    let nonce = nonce()?;
    let answer = state
        .prover
        .fetch(domain, &nonce, &allowed)
        .await
        .map_err(|why| {
            tracing::info!(%domain, %why, "domain proof not obtained");
            refused(
                "The domain did not answer the authority's check. Is the panel reachable there?",
            )
        })?;
    if answer.installation_id != installation_id
        || crate::binding_name(&answer.domain) != crate::binding_name(domain)
    {
        return Err(refused(
            "The domain answered for another installation or domain.",
        ));
    }
    public::check_signature(
        public_key,
        &public::proof_message(
            "domain",
            &nonce,
            installation_id,
            Some(&crate::binding_name(domain)),
        ),
        &answer.signature,
    )
    .map_err(|_| refused("The domain's answer is not signed by this installation."))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_long_and_never_repeat() {
        let a = nonce().unwrap();
        let b = nonce().unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
    }
}
