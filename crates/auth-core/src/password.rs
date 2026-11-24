//! Argon2id password hashing.
//!
//! Hashes are stored as PHC strings (`$argon2id$v=19$m=…,t=…,p=…$salt$hash`),
//! so each hash carries its own parameters. [`Verification::needs_rehash`]
//! tells the caller to re-hash after a successful login when the stored
//! parameters are weaker than the current configuration.
//!
//! Hashing is CPU- and memory-heavy by design. In async code use
//! [`PasswordHasher::hash_async`] / [`PasswordHasher::verify_async`], which
//! run on Tokio's blocking pool so request handling is not stalled.

use std::sync::Arc;

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher as _, PasswordVerifier as _, phc::PasswordHash as PhcHash},
};
use secrecy::{ExposeSecret, SecretString};

/// OWASP minimum for argon2id with p=1 is m=19 MiB, t=2. Anything weaker is refused.
pub const MIN_MEMORY_KIB: u32 = 19 * 1024;
/// Minimum iterations accepted.
pub const MIN_ITERATIONS: u32 = 2;

/// Argon2id cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// Memory cost in KiB.
    pub memory_kib: u32,
    /// Number of passes.
    pub iterations: u32,
    /// Degree of parallelism (lanes).
    pub parallelism: u32,
}

impl Default for Argon2Params {
    /// OWASP baseline for argon2id: 19 MiB, 2 passes, 1 lane.
    ///
    /// Measured ~470 ms per hash on the 4-core 2019 Intel dev Mac under load;
    /// expected well under that on current server CPUs. Raise via
    /// `auth.argon2_*` after benchmarking the target host (see
    /// `benchmark_argon2_params`); stored hashes upgrade on next login.
    fn default() -> Self {
        Self {
            memory_kib: MIN_MEMORY_KIB,
            iterations: MIN_ITERATIONS,
            parallelism: 1,
        }
    }
}

/// Errors from hashing and verification. They never contain the password.
#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    /// Parameters are below the accepted minimum or otherwise invalid.
    #[error("argon2 parameters rejected: {0}")]
    WeakParams(String),
    /// Hashing failed (resource exhaustion or library error).
    #[error("password hashing failed: {0}")]
    Hash(String),
    /// The stored hash is not a valid argon2id PHC string.
    #[error("stored password hash is malformed")]
    MalformedHash,
    /// The blocking task panicked or was cancelled.
    #[error("password task failed: {0}")]
    Task(String),
}

/// Outcome of a verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verification {
    /// Whether the password matched.
    pub matches: bool,
    /// True if the password matched but the stored hash uses older/weaker
    /// parameters than the current configuration; re-hash and store.
    pub needs_rehash: bool,
}

/// Hashes and verifies passwords with fixed argon2id parameters. Cheap to clone.
#[derive(Clone)]
pub struct PasswordHasher {
    inner: Arc<Inner>,
}

struct Inner {
    params: Argon2Params,
    argon2: Argon2<'static>,
    /// Hash of a random value, verified against when the account does not
    /// exist so both paths cost the same time.
    dummy_hash: String,
}

impl std::fmt::Debug for PasswordHasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordHasher")
            .field("params", &self.inner.params)
            .finish_non_exhaustive()
    }
}

impl PasswordHasher {
    /// Builds a hasher. Computes one hash up front for the dummy verifier.
    ///
    /// # Errors
    /// [`PasswordError::WeakParams`] below the OWASP minimum or if argon2 rejects them.
    pub fn new(params: Argon2Params) -> Result<Self, PasswordError> {
        if params.memory_kib < MIN_MEMORY_KIB {
            return Err(PasswordError::WeakParams(format!(
                "memory {} KiB is below the minimum {MIN_MEMORY_KIB} KiB",
                params.memory_kib
            )));
        }
        if params.iterations < MIN_ITERATIONS {
            return Err(PasswordError::WeakParams(format!(
                "iterations {} is below the minimum {MIN_ITERATIONS}",
                params.iterations
            )));
        }
        let argon_params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            None,
        )
        .map_err(|e| PasswordError::WeakParams(e.to_string()))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

        let dummy_password = argon2::password_hash::generate_salt();
        let dummy_hash = argon2
            .hash_password(&dummy_password)
            .map_err(|e| PasswordError::Hash(e.to_string()))?
            .to_string();

        Ok(Self {
            inner: Arc::new(Inner {
                params,
                argon2,
                dummy_hash,
            }),
        })
    }

    /// The configured parameters.
    #[must_use]
    pub fn params(&self) -> Argon2Params {
        self.inner.params
    }

    /// Hashes a password with a fresh random salt. Blocking; see [`Self::hash_async`].
    ///
    /// # Errors
    /// [`PasswordError::Hash`] if argon2 fails.
    pub fn hash(&self, password: &SecretString) -> Result<String, PasswordError> {
        self.inner
            .argon2
            .hash_password(password.expose_secret().as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| PasswordError::Hash(e.to_string()))
    }

    /// Verifies a password against a stored PHC string. Blocking; see [`Self::verify_async`].
    ///
    /// # Errors
    /// [`PasswordError::MalformedHash`] if the stored value is not argon2id PHC.
    pub fn verify(
        &self,
        password: &SecretString,
        stored: &str,
    ) -> Result<Verification, PasswordError> {
        let parsed = PhcHash::new(stored).map_err(|_| PasswordError::MalformedHash)?;
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(PasswordError::MalformedHash);
        }
        let stored_params = Params::try_from(&parsed).map_err(|_| PasswordError::MalformedHash)?;
        // Parameters come from the stored hash, not from `self`.
        let matches = self
            .inner
            .argon2
            .verify_password(password.expose_secret().as_bytes(), &parsed)
            .is_ok();
        let current = self.inner.params;
        let needs_rehash = matches
            && (stored_params.m_cost() < current.memory_kib
                || stored_params.t_cost() < current.iterations
                || stored_params.p_cost() != current.parallelism);
        Ok(Verification {
            matches,
            needs_rehash,
        })
    }

    /// Spends the same time as a real verification and always returns `false`.
    /// Call it when the username does not exist, so response timing does not
    /// reveal which accounts exist.
    #[must_use]
    pub fn verify_dummy(&self, password: &SecretString) -> bool {
        let _ = self.verify(password, &self.inner.dummy_hash);
        false
    }

    /// [`Self::hash`] on the blocking thread pool.
    ///
    /// # Errors
    /// As [`Self::hash`], plus [`PasswordError::Task`] if the task fails.
    pub async fn hash_async(&self, password: SecretString) -> Result<String, PasswordError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.hash(&password))
            .await
            .map_err(|e| PasswordError::Task(e.to_string()))?
    }

    /// [`Self::verify`] on the blocking thread pool.
    ///
    /// # Errors
    /// As [`Self::verify`], plus [`PasswordError::Task`] if the task fails.
    pub async fn verify_async(
        &self,
        password: SecretString,
        stored: String,
    ) -> Result<Verification, PasswordError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.verify(&password, &stored))
            .await
            .map_err(|e| PasswordError::Task(e.to_string()))?
    }

    /// [`Self::verify_dummy`] on the blocking thread pool.
    pub async fn verify_dummy_async(&self, password: SecretString) -> bool {
        let this = self.clone();
        let _ = tokio::task::spawn_blocking(move || this.verify_dummy(&password)).await;
        false
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Cheapest accepted parameters keep the unit tests fast.
    fn fast() -> PasswordHasher {
        PasswordHasher::new(Argon2Params {
            memory_kib: MIN_MEMORY_KIB,
            iterations: MIN_ITERATIONS,
            parallelism: 1,
        })
        .expect("valid params")
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn hash_is_argon2id_phc_with_configured_params_and_random_salt() {
        let h = fast();
        let a = h
            .hash(&secret("correct horse battery staple"))
            .expect("hash");
        let b = h
            .hash(&secret("correct horse battery staple"))
            .expect("hash");
        assert!(a.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{a}");
        assert_ne!(a, b, "salts must differ");
        assert!(!a.contains("correct horse"));
    }

    #[test]
    fn verify_accepts_right_and_rejects_wrong_password() {
        let h = fast();
        let stored = h.hash(&secret("s3cure-Passphrase!")).expect("hash");
        assert_eq!(
            h.verify(&secret("s3cure-Passphrase!"), &stored)
                .expect("ok"),
            Verification {
                matches: true,
                needs_rehash: false
            }
        );
        let wrong = h
            .verify(&secret("s3cure-passphrase!"), &stored)
            .expect("ok");
        assert!(!wrong.matches);
        assert!(!wrong.needs_rehash);
    }

    #[test]
    fn weaker_stored_params_request_rehash_only_on_match() {
        let old = fast();
        let stored = old.hash(&secret("Migrate-me-please-42")).expect("hash");
        let current = PasswordHasher::new(Argon2Params {
            memory_kib: MIN_MEMORY_KIB * 2,
            iterations: 3,
            parallelism: 1,
        })
        .expect("valid");
        let ok = current
            .verify(&secret("Migrate-me-please-42"), &stored)
            .expect("ok");
        assert!(ok.matches && ok.needs_rehash);
        let bad = current.verify(&secret("nope"), &stored).expect("ok");
        assert!(!bad.matches && !bad.needs_rehash);
    }

    #[test]
    fn malformed_or_non_argon2id_hashes_are_rejected() {
        let h = fast();
        for stored in [
            "hunter2",
            "",
            "$argon2i$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA",
            "$2b$12$abcdefghijklmnopqrstuuR1g9z1ZQ9oTn0Yc6eP2iVvM3kq8m5G2",
        ] {
            assert!(
                matches!(
                    h.verify(&secret("x"), stored),
                    Err(PasswordError::MalformedHash)
                ),
                "{stored}"
            );
        }
    }

    #[test]
    fn params_below_owasp_minimum_are_refused() {
        let err = PasswordHasher::new(Argon2Params {
            memory_kib: 8 * 1024,
            iterations: 3,
            parallelism: 1,
        })
        .expect_err("too little memory");
        assert!(matches!(err, PasswordError::WeakParams(_)));
        let err = PasswordHasher::new(Argon2Params {
            memory_kib: MIN_MEMORY_KIB,
            iterations: 1,
            parallelism: 1,
        })
        .expect_err("too few iterations");
        assert!(matches!(err, PasswordError::WeakParams(_)));
    }

    #[test]
    fn dummy_verify_always_fails_and_costs_about_as_much_as_a_real_one() {
        let h = fast();
        let stored = h.hash(&secret("Real-password-123")).expect("hash");
        let t = Instant::now();
        let _ = h.verify(&secret("guess"), &stored);
        let real = t.elapsed();
        let t = Instant::now();
        assert!(!h.verify_dummy(&secret("guess")));
        let dummy = t.elapsed();
        // Same parameters, same work: allow generous scheduling noise.
        assert!(
            dummy * 3 > real && real * 3 > dummy,
            "real={real:?} dummy={dummy:?}"
        );
    }

    #[test]
    fn debug_does_not_expose_dummy_hash() {
        let dbg = format!("{:?}", fast());
        assert!(!dbg.contains("$argon2id$"), "{dbg}");
    }

    #[tokio::test]
    async fn async_variants_match_sync_results() {
        let h = fast();
        let stored = h
            .hash_async(secret("Async-pass-Word-9"))
            .await
            .expect("hash");
        let ok = h
            .verify_async(secret("Async-pass-Word-9"), stored.clone())
            .await
            .expect("verify");
        assert!(ok.matches);
        assert!(!h.verify_dummy_async(secret("x")).await);
    }

    /// Measures one hash. Release builds only (debug argon2 is ~10x slower and
    /// would make `--include-ignored` integration runs crawl).
    ///
    /// Calibrate on the target server:
    /// `BUILTPANEL_BENCH_ARGON2=65536,3,1 cargo test -p builtpanel-auth-core --release -- --ignored --nocapture benchmark`
    /// (memory KiB, iterations, parallelism; defaults to the production defaults).
    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "benchmark; run explicitly with --release --ignored --nocapture"]
    fn benchmark_argon2_params() {
        let params = std::env::var("BUILTPANEL_BENCH_ARGON2")
            .ok()
            .and_then(|v| {
                let n: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (n.len() == 3).then(|| Argon2Params {
                    memory_kib: n[0],
                    iterations: n[1],
                    parallelism: n[2],
                })
            })
            .unwrap_or_default();
        let h = PasswordHasher::new(params).expect("valid params");
        let pw = secret("Benchmark-Password-2026!");
        let _ = h.hash(&pw).expect("warm-up");
        let runs: u32 = 5;
        let t = Instant::now();
        for _ in 0..runs {
            let _ = h.hash(&pw).expect("hash");
        }
        let avg = t.elapsed() / runs;
        println!(
            "argon2id m={}KiB t={} p={}: {avg:?} per hash (avg of {runs})",
            params.memory_kib, params.iterations, params.parallelism
        );
    }
}
