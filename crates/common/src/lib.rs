//! Shared primitives used across Built Panel crates and apps.
//!
//! Later tasks add ID types, time helpers, pagination and result types here.

/// Product name shown in logs and `--version` output.
pub const PRODUCT_NAME: &str = "Built Panel";

/// Workspace version, taken from `Cargo.toml` at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns a one-line identity string such as `Built Panel control-api 0.1.0`.
#[must_use]
pub fn build_banner(component: &str) -> String {
    format!("{PRODUCT_NAME} {component} {VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_contains_product_component_and_version() {
        let banner = build_banner("worker");
        assert_eq!(banner, format!("Built Panel worker {VERSION}"));
    }

    #[test]
    fn version_is_semver_like() {
        assert_eq!(
            VERSION.split('.').count(),
            3,
            "unexpected version: {VERSION}"
        );
    }
}
