//! The wire format: one JSON line each way.

use serde::{Deserialize, Serialize};

/// Largest request line accepted, in bytes.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// One default file of a new tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonFile {
    /// Path below the tenant's `www`, with `/` between names.
    pub path: String,
    /// The file's text.
    pub content: String,
}

/// Everything the helper can be asked to do. Adding a variant here is the only
/// way to add a privileged action, and each one is reviewed on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Does nothing; proves that the channel and the permission check work.
    Noop,
    /// Creates a tenant's directories, owned by the tenant's host id
    /// (see [`crate::tenant`]).
    PrepareTenant {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id inside the user namespace (10000..59999).
        uid: u32,
        /// Default files for the tenant's `www` directory (see [`crate::skeleton`]).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skeleton: Vec<SkeletonFile>,
    },
    /// Sets the disk and file-count limits of a tenant (see [`crate::quota`]).
    /// `None` means unlimited.
    SetStorageQuota {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (also its quota project id).
        uid: u32,
        /// Most bytes the tenant's directory may hold.
        disk_bytes: Option<u64>,
        /// Most files and directories.
        inodes: Option<u64>,
    },
    /// Sets which IPv4 addresses and ranges may reach the database server's public
    /// port (see [`crate::firewall`]); everything else aimed at that port is dropped.
    /// An empty list closes it for everybody. The port is the helper's own setting,
    /// not part of the request.
    SetDbFirewall {
        /// `a.b.c.d/n`, n from 8 to 32, at most 2000.
        cidrs: Vec<String>,
    },
    /// [`Action::SetDbFirewall`] for the PostgreSQL server's public port (its own table and
    /// its own port, the helper's setting `--pg-port`).
    SetPgFirewall {
        /// `a.b.c.d/n`, n from 8 to 32, at most 2000.
        cidrs: Vec<String>,
    },
    /// Does one file manager request on a tenant's files, as the tenant's own user
    /// id, in a process of its own (see [`crate::files`]).
    FileOp {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
        /// What to do.
        request: builtpanel_node_protocol::files::FileRequest,
    },
    /// Puts a staged file into a tenant's files as the tenant's own user: written
    /// beside its place and moved in, never over something unless `overwrite`.
    PlaceFile {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
        /// The staged file: a plain name in the agent's transfer folder.
        staged: String,
        /// Where it goes, relative to the tenant's files.
        dest: String,
        /// Replace a file that is there.
        overwrite: bool,
        /// Refuse a file bigger than this.
        max_bytes: u64,
    },
    /// Copies a file of a tenant's, read as the tenant's own user, to a new staged
    /// file in the agent's transfer folder.
    TakeFile {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
        /// The file, relative to the tenant's files.
        path: String,
        /// The staged file to make: a plain name in the agent's transfer folder.
        staged: String,
        /// Refuse a file bigger than this.
        max_bytes: u64,
    },
    /// Packs a tenant's files, as the tenant's own user, into an encrypted stream in a new
    /// staged file of the agent's transfer folder (see `builtpanel_fileops::backup`). The key
    /// (64 hex digits) goes to the worker through its input, never through a command line.
    BackupFiles {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
        /// The staged file to make: a plain name in the agent's transfer folder.
        staged: String,
        /// The key.
        key: String,
        /// Only files made or changed after this time (Unix seconds).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<i64>,
    },
    /// Puts the files of an encrypted backup stream (a staged file of the agent's transfer
    /// folder, see [`Action::BackupFiles`]) back into a tenant's folder, as the tenant's own
    /// user: over what is there, nothing removed. The key goes to the worker through its input.
    RestoreFiles {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
        /// The staged file to read: a plain name in the agent's transfer folder.
        staged: String,
        /// The key (64 hex digits).
        key: String,
        /// Only this folder (relative to the tenant's files); everything when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
    },
    /// Replaces the node's firewall table `builtpanel_fw` with the one the policy describes (see
    /// [`crate::nodefw`]): checked with the packet filter first, applied in one transaction.
    SetNodeFirewall {
        /// The policy (checked again here).
        policy: builtpanel_node_protocol::FirewallPolicy,
    },
    /// Sets the machine's time zone (a zone file of this machine; see [`crate::servertools`]).
    SetTimezone {
        /// `Area/City`.
        zone: String,
    },
    /// Makes the swap file this many MiB big, replacing the old one; 0 takes it away.
    SetSwap {
        /// MiB, 0 to 32768.
        megabytes: u32,
    },
    /// Writes dirty pages to disk and drops the page cache.
    DropCaches,
    /// Reboots the machine.
    Reboot,
    /// Removes a tenant's two (empty) directories after its files were
    /// deleted. Never deletes any file: a non-empty directory is a failure.
    RemoveTenant {
        /// The tenant's id (a lower-case UUID).
        tenant_id: String,
        /// The tenant's numeric id (10000..59999).
        uid: u32,
    },
}

impl Action {
    /// The action's wire name (safe to log).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::PrepareTenant { .. } => "prepare_tenant",
            Self::SetStorageQuota { .. } => "set_storage_quota",
            Self::SetDbFirewall { .. } => "set_db_firewall",
            Self::SetPgFirewall { .. } => "set_pg_firewall",
            Self::FileOp { .. } => "file_op",
            Self::PlaceFile { .. } => "place_file",
            Self::TakeFile { .. } => "take_file",
            Self::BackupFiles { .. } => "backup_files",
            Self::RestoreFiles { .. } => "restore_files",
            Self::SetNodeFirewall { .. } => "set_node_firewall",
            Self::SetTimezone { .. } => "set_timezone",
            Self::SetSwap { .. } => "set_swap",
            Self::DropCaches => "drop_caches",
            Self::Reboot => "reboot",
            Self::RemoveTenant { .. } => "remove_tenant",
        }
    }
}

/// A request from the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Chosen by the caller, echoed in the answer and the log (1-64 characters
    /// of `A-Z a-z 0-9 . _ : -`).
    pub id: String,
    /// What to do.
    pub action: Action,
}

/// Why a request was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Not JSON, not a request, too long, or a bad id.
    InvalidRequest,
    /// The action is not on the list.
    UnknownAction,
    /// The action is known but its arguments are not acceptable.
    InvalidArguments,
    /// The caller is not the agent.
    Forbidden,
    /// The action was attempted and failed.
    Failed,
    /// This machine cannot do it (for example: no project quotas on its filesystem).
    Unsupported,
}

/// The helper's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// The request's id (empty when the request could not be read).
    pub id: String,
    /// Whether the action was carried out.
    pub ok: bool,
    /// Present when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
    /// What the action has to say, when it has something (a file manager answer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Failure detail. Messages are fixed texts, never echoes of the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseError {
    /// Machine-readable reason.
    pub code: ErrorCode,
    /// Human-readable reason.
    pub message: String,
}

impl Response {
    /// Success.
    #[must_use]
    pub fn done(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            ok: true,
            error: None,
            data: None,
        }
    }

    /// Success with something to say.
    #[must_use]
    pub fn with_data(id: &str, data: serde_json::Value) -> Self {
        Self {
            data: Some(data),
            ..Self::done(id)
        }
    }

    /// Refusal or failure.
    #[must_use]
    pub fn refused(id: &str, code: ErrorCode, message: &str) -> Self {
        Self {
            id: id.to_owned(),
            ok: false,
            error: Some(ResponseError {
                code,
                message: message.to_owned(),
            }),
            data: None,
        }
    }
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// The keys an action object may have (including `type`), or `None` for an
/// action that is not on the list. Must be extended together with [`Action`].
fn allowed_fields(action: &str) -> Option<&'static [&'static str]> {
    match action {
        "noop" => Some(&["type"]),
        "prepare_tenant" => Some(&["type", "tenant_id", "uid", "skeleton"]),
        "set_storage_quota" => Some(&["type", "tenant_id", "uid", "disk_bytes", "inodes"]),
        "set_db_firewall" => Some(&["type", "cidrs"]),
        "file_op" => Some(&["type", "tenant_id", "uid", "request"]),
        "place_file" => Some(&[
            "type",
            "tenant_id",
            "uid",
            "staged",
            "dest",
            "overwrite",
            "max_bytes",
        ]),
        "take_file" => Some(&["type", "tenant_id", "uid", "path", "staged", "max_bytes"]),
        "backup_files" => Some(&["type", "tenant_id", "uid", "staged", "key"]),
        "remove_tenant" => Some(&["type", "tenant_id", "uid"]),
        _ => None,
    }
}

/// Why a line is not a request, with what may be logged about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// The answer to send.
    pub response: Response,
    /// The action name the caller asked for, when it could be read and is
    /// harmless to log (letters, digits, `_`; at most 64 characters).
    pub asked_for: Option<String>,
}

/// Reads one request line.
///
/// # Errors
/// [`Rejected`] with the answer to send back.
pub fn parse_request(line: &[u8]) -> Result<Request, Rejected> {
    let reject = |id: &str, code, message: &str, asked_for: Option<String>| Rejected {
        response: Response::refused(id, code, message),
        asked_for,
    };
    if line.len() > MAX_REQUEST_BYTES {
        return Err(reject(
            "",
            ErrorCode::InvalidRequest,
            "request too large",
            None,
        ));
    }
    // Look at the shape first, so that an unknown action can be told apart
    // from garbage and named in the log.
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
        return Err(reject("", ErrorCode::InvalidRequest, "not JSON", None));
    };
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|id| valid_id(id))
        .unwrap_or_default()
        .to_owned();
    if id.is_empty() {
        return Err(reject(
            "",
            ErrorCode::InvalidRequest,
            "missing or invalid id",
            None,
        ));
    }
    let asked_for = value
        .get("action")
        .and_then(|a| a.get("type"))
        .and_then(|t| t.as_str())
        .filter(|t| {
            (1..=64).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(str::to_owned);
    // serde's `deny_unknown_fields` does not cover every enum shape (a variant
    // without fields accepts extra keys), so the allowed keys are checked here.
    let extra_keys = match (&asked_for, value.get("action").and_then(|a| a.as_object())) {
        (Some(name), Some(fields)) => allowed_fields(name)
            .is_some_and(|allowed| fields.keys().any(|k| !allowed.contains(&k.as_str()))),
        _ => false,
    };
    if extra_keys {
        return Err(reject(
            &id,
            ErrorCode::InvalidArguments,
            "invalid arguments for this action",
            asked_for,
        ));
    }
    match serde_json::from_value::<Request>(value) {
        Ok(request) => Ok(request),
        Err(_) => match &asked_for {
            Some(name) if allowed_fields(name).is_some() => Err(reject(
                &id,
                ErrorCode::InvalidArguments,
                "invalid arguments for this action",
                asked_for,
            )),
            Some(_) => Err(reject(
                &id,
                ErrorCode::UnknownAction,
                "this action is not on the allow-list",
                asked_for,
            )),
            None => Err(reject(
                &id,
                ErrorCode::InvalidRequest,
                "not a request",
                None,
            )),
        },
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_noop_request_round_trips() {
        let line = br#"{"id":"req-1","action":{"type":"noop"}}"#;
        let request = parse_request(line).unwrap();
        assert_eq!(
            request,
            Request {
                id: "req-1".into(),
                action: Action::Noop
            }
        );
        assert_eq!(serde_json::to_string(&request).unwrap().as_bytes(), line);
        assert_eq!(
            serde_json::to_string(&Response::done("req-1")).unwrap(),
            r#"{"id":"req-1","ok":true}"#
        );
    }

    #[test]
    fn unknown_actions_are_refused_and_named() {
        for (line, name) in [
            (
                r#"{"id":"a","action":{"type":"run_command","cmd":"id"}}"#,
                "run_command",
            ),
            (r#"{"id":"a","action":{"type":"shell"}}"#, "shell"),
            (r#"{"id":"a","action":{"type":"Noop"}}"#, "Noop"),
        ] {
            let rejected = parse_request(line.as_bytes()).unwrap_err();
            assert_eq!(
                rejected.response.error.as_ref().unwrap().code,
                ErrorCode::UnknownAction,
                "{line}"
            );
            assert_eq!(rejected.response.id, "a");
            assert_eq!(rejected.asked_for.as_deref(), Some(name));
            assert!(!rejected.response.ok);
        }
    }

    #[test]
    fn extra_fields_and_bad_shapes_are_refused() {
        let code = |line: &str| {
            parse_request(line.as_bytes())
                .unwrap_err()
                .response
                .error
                .unwrap()
                .code
        };
        // A known action with something extra is not silently accepted.
        assert_eq!(
            code(r#"{"id":"a","action":{"type":"noop","cmd":"id"}}"#),
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            code(r#"{"id":"a","action":{"type":"noop"},"sudo":true}"#),
            ErrorCode::InvalidArguments
        );
        assert_eq!(code(r#"{"id":"a"}"#), ErrorCode::InvalidRequest);
        assert_eq!(
            code(r#"{"id":"a","action":"noop"}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code(r#"{"action":{"type":"noop"}}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code(r#"{"id":"","action":{"type":"noop"}}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code(r#"{"id":"a b","action":{"type":"noop"}}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code(r#"{"id":7,"action":{"type":"noop"}}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(code("not json"), ErrorCode::InvalidRequest);
        assert_eq!(code(""), ErrorCode::InvalidRequest);
        assert_eq!(code("[]"), ErrorCode::InvalidRequest);
        let huge = format!(
            r#"{{"id":"a","action":{{"type":"noop"}},"pad":"{}"}}"#,
            "x".repeat(MAX_REQUEST_BYTES)
        );
        assert_eq!(code(&huge), ErrorCode::InvalidRequest);
    }

    #[test]
    fn hostile_action_names_are_not_passed_to_the_log() {
        for line in [
            r#"{"id":"a","action":{"type":"x\nFAKE LOG LINE"}}"#.to_owned(),
            r#"{"id":"a","action":{"type":"a b"}}"#.to_owned(),
            format!(r#"{{"id":"a","action":{{"type":"{}"}}}}"#, "x".repeat(65)),
        ] {
            let rejected = parse_request(line.as_bytes()).unwrap_err();
            assert_eq!(rejected.asked_for, None, "{line}");
        }
    }
}
