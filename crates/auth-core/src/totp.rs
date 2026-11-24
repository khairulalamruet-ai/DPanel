//! Time-based one-time passwords (RFC 6238) and backup codes for 2FA.
//!
//! * TOTP: HMAC-SHA1, 6 digits, 30-second steps, one step of clock skew either
//!   way — the parameters every authenticator app supports.
//! * A matched code yields its time step; the caller must store the last
//!   accepted step and refuse steps that are not newer (replay protection).
//! * Backup codes: ten single-use codes with ~49.5 bits of entropy each, shown
//!   once and stored only as SHA-256 hashes (they are random, so a fast hash
//!   is sufficient).

use sha2::{Digest as _, Sha256};
use totp_rs::{Builder, Secret, Totp};

/// TOTP secret length in bytes (160 bits, the RFC 4226 recommendation).
pub const SECRET_LEN: usize = 20;
/// Digits in a TOTP code.
pub const CODE_DIGITS: usize = 6;
/// Seconds per time step.
pub const STEP_SECONDS: u64 = 30;
/// Number of backup codes issued at enrolment.
pub const BACKUP_CODE_COUNT: usize = 10;
/// Characters in a backup code, not counting the separator.
pub const BACKUP_CODE_CHARS: usize = 10;
/// 31 unambiguous characters (no `0/o`, `1/l/i`).
const BACKUP_ALPHABET: &[u8; 31] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// Errors from this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TotpError {
    /// OS randomness unavailable.
    #[error("random number generator unavailable")]
    Rng,
    /// The stored secret has the wrong length or the label is unusable.
    #[error("invalid TOTP parameters")]
    Invalid,
}

/// A TOTP shared secret. Redacted in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct TotpSecret([u8; SECRET_LEN]);

impl std::fmt::Debug for TotpSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TotpSecret(<redacted>)")
    }
}

impl TotpSecret {
    /// A new random secret.
    ///
    /// # Errors
    /// [`TotpError::Rng`].
    pub fn generate() -> Result<Self, TotpError> {
        let mut bytes = [0u8; SECRET_LEN];
        getrandom::fill(&mut bytes).map_err(|_| TotpError::Rng)?;
        Ok(Self(bytes))
    }

    /// From stored bytes.
    ///
    /// # Errors
    /// [`TotpError::Invalid`] unless exactly [`SECRET_LEN`] bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TotpError> {
        bytes.try_into().map(Self).map_err(|_| TotpError::Invalid)
    }

    /// Raw bytes, for encryption at rest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }

    /// Base32 form for manual entry into an authenticator app.
    #[must_use]
    pub fn to_base32(&self) -> String {
        Secret::from(self.0).to_base32()
    }

    fn totp(&self, issuer: &str, account: &str) -> Result<Totp, TotpError> {
        Builder::new()
            .with_secret(Secret::from(self.0))
            .with_issuer(Some(issuer.replace(':', " ")))
            .with_account_name(account.replace(':', " "))
            .build()
            .map_err(|_| TotpError::Invalid)
    }

    /// `otpauth://totp/...` URI to show as a QR code.
    ///
    /// # Errors
    /// [`TotpError::Invalid`] if the label cannot be encoded.
    pub fn otpauth_uri(&self, issuer: &str, account: &str) -> Result<String, TotpError> {
        self.totp(issuer, account)?
            .to_url()
            .map_err(|_| TotpError::Invalid)
    }

    /// Checks a 6-digit code at `unix_time`, allowing one step of skew.
    /// Returns the matched time step; the caller must reject steps it has
    /// already accepted. Spaces in the input are ignored.
    #[must_use]
    pub fn verify(&self, code: &str, unix_time: u64) -> Option<u64> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != CODE_DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.totp("x", "x").ok()?.check(&code, unix_time)
    }

    /// The code valid at `unix_time` (for tests and tooling).
    #[must_use]
    pub fn code_at(&self, unix_time: u64) -> String {
        self.totp("x", "x")
            .map(|t| t.generate(unix_time).to_string())
            .unwrap_or_default()
    }
}

/// A freshly generated backup code (shown to the user exactly once).
#[derive(Clone, PartialEq, Eq)]
pub struct BackupCode(String);

impl std::fmt::Debug for BackupCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupCode(<redacted>)")
    }
}

impl BackupCode {
    /// Display form, e.g. `k7m2p-9xqrt`.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Hash to store.
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        hash_backup_code(&self.0).unwrap_or([0; 32])
    }
}

/// Generates [`BACKUP_CODE_COUNT`] distinct backup codes.
///
/// # Errors
/// [`TotpError::Rng`].
pub fn generate_backup_codes() -> Result<Vec<BackupCode>, TotpError> {
    let mut codes: Vec<BackupCode> = Vec::with_capacity(BACKUP_CODE_COUNT);
    // Bytes at or above this would bias `% 31`; they are drawn again.
    const UNBIASED_LIMIT: u8 = 31 * 8;
    while codes.len() < BACKUP_CODE_COUNT {
        let mut text = String::with_capacity(BACKUP_CODE_CHARS + 1);
        while text.len() < BACKUP_CODE_CHARS + 1 {
            let mut raw = [0u8; 16];
            getrandom::fill(&mut raw).map_err(|_| TotpError::Rng)?;
            for byte in raw.into_iter().filter(|b| *b < UNBIASED_LIMIT) {
                if text.len() == BACKUP_CODE_CHARS / 2 {
                    text.push('-');
                }
                if text.len() < BACKUP_CODE_CHARS + 1 {
                    text.push(char::from(BACKUP_ALPHABET[usize::from(byte % 31)]));
                }
            }
        }
        let code = BackupCode(text);
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    Ok(codes)
}

/// Normalises user input (case, spaces, dashes) and hashes it. `None` when the
/// input cannot be a backup code, so garbage never reaches the database.
#[must_use]
pub fn hash_backup_code(input: &str) -> Option<[u8; 32]> {
    let normalised: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if normalised.len() != BACKUP_CODE_CHARS
        || !normalised.bytes().all(|b| BACKUP_ALPHABET.contains(&b))
    {
        return None;
    }
    Some(Sha256::digest(normalised.as_bytes()).into())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B test secret ("12345678901234567890").
    fn rfc_secret() -> TotpSecret {
        TotpSecret::from_bytes(b"12345678901234567890").unwrap()
    }

    #[test]
    fn matches_rfc_6238_test_vectors() {
        // SHA-1 vectors truncated to 6 digits.
        for (time, code) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            assert_eq!(rfc_secret().code_at(time), code, "t={time}");
            assert_eq!(rfc_secret().verify(code, time), Some(time / 30), "t={time}");
        }
    }

    #[test]
    fn accepts_one_step_of_skew_and_reports_the_matched_step() {
        let s = rfc_secret();
        let now = 1_234_567_890;
        let step = now / 30;
        assert_eq!(s.verify(&s.code_at(now - 30), now), Some(step - 1));
        assert_eq!(s.verify(&s.code_at(now + 30), now), Some(step + 1));
        assert_eq!(s.verify(&s.code_at(now - 60), now), None);
        assert_eq!(s.verify(&s.code_at(now + 60), now), None);
        assert_eq!(s.verify(" 005 924 ", now), Some(step));
    }

    #[test]
    fn rejects_malformed_codes() {
        let s = rfc_secret();
        for bad in ["", "12345", "1234567", "abcdef", "00592x", "-05924"] {
            assert_eq!(s.verify(bad, 1_234_567_890), None, "{bad}");
        }
    }

    #[test]
    fn secrets_are_random_and_round_trip() {
        let a = TotpSecret::generate().unwrap();
        let b = TotpSecret::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(TotpSecret::from_bytes(a.as_bytes()).unwrap(), a);
        assert_eq!(a.to_base32().len(), 32);
        assert!(TotpSecret::from_bytes(&[0; 19]).is_err());
        assert_eq!(format!("{a:?}"), "TotpSecret(<redacted>)");
    }

    #[test]
    fn otpauth_uri_carries_issuer_account_and_secret() {
        let s = rfc_secret();
        let uri = s.otpauth_uri("Built Panel", "khairul").unwrap();
        assert!(
            uri.starts_with("otpauth://totp/Built%20Panel:khairul?"),
            "{uri}"
        );
        assert!(uri.contains(&format!("secret={}", s.to_base32())), "{uri}");
        assert!(uri.contains("issuer=Built%20Panel"), "{uri}");
        // Colons would break the label; they are replaced.
        assert!(s.otpauth_uri("My:Panel", "a:b").is_ok());
    }

    #[test]
    fn backup_codes_are_distinct_well_formed_and_hash_consistently() {
        let codes = generate_backup_codes().unwrap();
        assert_eq!(codes.len(), BACKUP_CODE_COUNT);
        for (i, c) in codes.iter().enumerate() {
            let text = c.expose();
            assert_eq!(text.len(), 11, "{text}");
            assert_eq!(text.as_bytes()[5], b'-', "{text}");
            assert!(codes[..i].iter().all(|o| o != c));
            // Users may type it in upper case, with spaces or without the dash.
            let typed = text.to_uppercase().replace('-', " ");
            assert_eq!(hash_backup_code(&typed), Some(c.hash()));
        }
        assert_eq!(format!("{:?}", codes[0]), "BackupCode(<redacted>)");
    }

    #[test]
    fn only_plausible_backup_codes_are_hashed() {
        for bad in [
            "",
            "123456",
            "short",
            "abcde-fghj",
            "abcde-fghjkk",
            "abcde-fgh0k",
            "ab!de-fghjk",
        ] {
            assert_eq!(hash_backup_code(bad), None, "{bad}");
        }
        assert!(hash_backup_code("abcde-fghjk").is_some());
    }
}
