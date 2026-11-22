//! Built Panel node agent.
//!
//! The agent runs on every hosting server and does exactly what the control
//! plane's typed operations ask (ADR 0004). It never runs a shell.
//!
//! * [`enroll`]: trade a one-time token for a certificate.
//! * [`verify`]: check, with no trust in the server, what enrolment returned.
//! * [`identity`]: keep the key, certificate and identity on disk safely.

pub mod enroll;
pub mod identity;
pub mod ops;
pub mod stream;
pub mod verify;
pub mod wafevents;
pub mod web;

/// Version of this agent build, reported to the control plane.
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
