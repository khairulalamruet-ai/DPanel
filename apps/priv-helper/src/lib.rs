//! The privileged helper (T02.15).
//!
//! The node agent runs without privileges. The few host changes that need root
//! (firewall rules, quotas, ... in later phases) are asked of this small
//! program over a Unix socket. It is written to be read in one sitting:
//!
//! * **One caller.** The socket is owned by the agent's user with mode 0600,
//!   and the peer's uid is checked again on every connection (`SO_PEERCRED`).
//! * **A closed list of actions.** A request is one line of JSON naming an
//!   action from [`Action`]. Anything else — an unknown action, an extra field,
//!   a malformed or oversized line — is refused and logged. There is no action
//!   that runs a command or takes a path to execute, and none will be added.
//! * **Its own validation.** The helper does not trust the agent: every
//!   action validates its arguments here, whatever the agent already checked.
//! * **Everything is logged** (to stderr, which systemd puts in the journal):
//!   who asked, what, and whether it was done. Arguments that could be secret
//!   are never logged.
//!
//! One request, one answer, then the connection is closed.

pub mod files;
pub mod firewall;
pub mod nodefw;
pub mod protocol;
pub mod quota;
pub mod server;
pub mod servertools;
pub mod skeleton;
pub mod sshaccess;
pub mod tenant;

pub use protocol::{Action, ErrorCode, Request, Response};
