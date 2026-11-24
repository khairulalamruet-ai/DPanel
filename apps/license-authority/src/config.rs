//! The authority's configuration (T17.01): its own database and its own file, separate from
//! the control plane's. Defaults, then `/etc/builtpanel/license-authority.toml` (or the file named
//! by `BUILTPANEL_LA_CONFIG`), then `BUILTPANEL_LA_*` environment variables (`__` between nested
//! keys).

use std::{net::SocketAddr, path::PathBuf};

use figment::{
    Figment,
    providers::{Env, Format as _, Serialized, Toml},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// The default place of the configuration file.
pub const DEFAULT_PATH: &str = "/etc/builtpanel/license-authority.toml";

/// Everything the authority reads at start.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaConfig {
    /// Where it listens.
    pub bind: SocketAddr,
    /// Its PostgreSQL (not the control plane's).
    #[serde(serialize_with = "serialize_secret")]
    pub database_url: SecretString,
    /// The file holding the licence signing key (mode 0600).
    pub signing_key_file: PathBuf,
    /// The key id written into what it signs.
    pub signing_kid: String,
    /// The name in the `iss` claim.
    pub issuer: String,
    /// The name in the `aud` claim.
    pub audience: String,
    /// How long an entitlement stays valid after it is issued, in hours (it should be longer
    /// than the offline grace the installations allow themselves).
    pub entitlement_hours: u32,
    /// How long an admin sign-in lasts, in hours.
    pub session_hours: u32,
    /// The reverse proxies in front of the authority (their addresses). The address of a
    /// request is read from `X-Forwarded-For` only when the connection is from one of these.
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Whether an installation's first activation must also prove, from outside, that its panel
    /// answers at the domain it claims. Needs the panel to be reachable on https at the
    /// license's bound address.
    pub require_domain_proof: bool,
    /// The shared secret payments webhooks are signed with. Without it the webhook answers 503.
    #[serde(default, serialize_with = "serialize_opt_secret")]
    pub payment_webhook_secret: Option<SecretString>,
    /// Days a subscription whose payment did not arrive stays in good standing before it is
    /// `grace` (the license keeps working).
    pub billing_past_due_days: u32,
    /// Days of `grace` before the license is suspended.
    pub billing_grace_days: u32,
    /// Days suspended before the subscription is `expired`.
    pub billing_suspended_days: u32,
    /// Days before the end of a period the customer is reminded.
    pub billing_renewal_notice_days: u32,
    /// Days a payment extends a subscription when the provider does not say.
    pub billing_period_days: u32,
    /// Log format: `json` or `pretty`.
    pub log_format: String,
    /// Log filter directives.
    pub log_filter: String,
}

/// Only the defaults are ever serialised (to seed the layered configuration); nothing prints it.
fn serialize_secret<S: serde::Serializer>(v: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    use secrecy::ExposeSecret as _;
    s.serialize_str(v.expose_secret())
}

fn serialize_opt_secret<S: serde::Serializer>(
    v: &Option<SecretString>,
    s: S,
) -> Result<S::Ok, S::Error> {
    use secrecy::ExposeSecret as _;
    match v {
        Some(v) => s.serialize_some(v.expose_secret()),
        None => s.serialize_none(),
    }
}

impl Default for LaConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 9440)),
            database_url: SecretString::from("postgres://builtpanel_la@127.0.0.1/builtpanel_la"),
            signing_key_file: PathBuf::from("/etc/builtpanel/la-signing.key"),
            signing_kid: "license-1".to_owned(),
            issuer: "builtpanel-la".to_owned(),
            audience: "builtpanel".to_owned(),
            entitlement_hours: 96,
            session_hours: 8,
            trusted_proxies: vec![
                std::net::IpAddr::from([127, 0, 0, 1]),
                std::net::IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
            ],
            require_domain_proof: false,
            payment_webhook_secret: None,
            billing_past_due_days: 3,
            billing_grace_days: 7,
            billing_suspended_days: 30,
            billing_renewal_notice_days: 7,
            billing_period_days: 30,
            log_format: "json".to_owned(),
            log_filter: "info".to_owned(),
        }
    }
}

/// Why the configuration could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Unreadable or wrong values.
    #[error("{0}")]
    Read(String),
    /// A value out of range.
    #[error("{0}")]
    Invalid(String),
}

impl LaConfig {
    /// Loads and checks the configuration.
    ///
    /// # Errors
    /// [`ConfigError`].
    pub fn load() -> Result<Self, ConfigError> {
        let path = std::env::var_os("BUILTPANEL_LA_CONFIG")
            .map_or_else(|| PathBuf::from(DEFAULT_PATH), PathBuf::from);
        Self::load_from(&path)
    }

    /// [`Self::load`] with the file given.
    ///
    /// # Errors
    /// [`ConfigError`].
    pub fn load_from(path: &std::path::Path) -> Result<Self, ConfigError> {
        let mut figment = Figment::from(Serialized::defaults(Self::default()));
        if path.exists() {
            figment = figment.merge(Toml::file(path));
        }
        let config: Self = figment
            .merge(
                Env::prefixed("BUILTPANEL_LA_")
                    .ignore(&["CONFIG"])
                    .split("__"),
            )
            .extract()
            .map_err(|e| ConfigError::Read(e.to_string()))?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> Result<(), ConfigError> {
        use secrecy::ExposeSecret as _;
        let bad = |m: &str| Err(ConfigError::Invalid(m.to_owned()));
        if !(1..=24 * 365).contains(&self.entitlement_hours) {
            return bad("entitlement_hours: 1 to 8760");
        }
        if !(1..=24 * 30).contains(&self.session_hours) {
            return bad("session_hours: 1 to 720");
        }
        if !["json", "pretty"].contains(&self.log_format.as_str()) {
            return bad("log_format: json or pretty");
        }
        if !self.database_url.expose_secret().starts_with("postgres") {
            return bad("database_url: a postgres:// address");
        }
        for (name, v, max) in [
            ("billing_past_due_days", self.billing_past_due_days, 90),
            ("billing_grace_days", self.billing_grace_days, 90),
            ("billing_suspended_days", self.billing_suspended_days, 365),
            (
                "billing_renewal_notice_days",
                self.billing_renewal_notice_days,
                90,
            ),
            ("billing_period_days", self.billing_period_days, 3660),
        ] {
            if !(1..=max).contains(&v) {
                return Err(ConfigError::Invalid(format!("{name}: 1 to {max}")));
            }
        }
        if self
            .payment_webhook_secret
            .as_ref()
            .is_some_and(|s| s.expose_secret().len() < 16)
        {
            return bad("payment_webhook_secret: at least 16 characters");
        }
        if self.issuer.is_empty() || self.audience.is_empty() {
            return bad("issuer and audience may not be empty");
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::result_large_err)]
    fn the_defaults_are_valid_and_the_environment_overrides_them() {
        figment::Jail::expect_with(|jail| {
            let c = LaConfig::load_from(std::path::Path::new("/nonexistent")).unwrap();
            assert_eq!(c.entitlement_hours, 96);
            jail.set_env("BUILTPANEL_LA_ENTITLEMENT_HOURS", "48");
            jail.set_env("BUILTPANEL_LA_DATABASE_URL", "postgres://x@db/la");
            let c = LaConfig::load_from(std::path::Path::new("/nonexistent")).unwrap();
            assert_eq!(c.entitlement_hours, 48);
            jail.set_env("BUILTPANEL_LA_ENTITLEMENT_HOURS", "0");
            assert!(LaConfig::load_from(std::path::Path::new("/nonexistent")).is_err());
            Ok(())
        });
    }
}
