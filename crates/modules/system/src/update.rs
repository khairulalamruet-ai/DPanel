//! The panel's own update (the admin area's "Update now").
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/update/status` | `admin.settings.read` |
//! | POST | `/api/v1/admin/update/check` | `admin.settings.manage` |
//! | POST | `/api/v1/admin/update/apply` | `admin.settings.manage`, sudo mode |
//!
//! The control plane never runs as root and does not install anything: it asks the release site which
//! version is the newest, and for "apply" it leaves a request (one version number) in a folder only it
//! can write. A root service of its own (`deploy/release/self-update.sh`) reads that number, fetches the
//! release, checks every file against the signed release manifest and installs it (migrating,
//! restarting, putting the old version back when the new one does not come up). What it did is in
//! `status.json` and `history.json`, written by root and only read here.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{FromRef, State},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::sqlx::{self, PgPool};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_kernel::Event;
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{SettingsManage, SettingsRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where releases are published unless `BUILTPANEL_UPDATE_REPO` says otherwise.
pub const DEFAULT_REPO: &str = "khairulalamruet-ai/DPanel";

/// The version this build is.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long the release site gets to answer.
const TIMEOUT: Duration = Duration::from_secs(20);

/// An hour-old check is no check: the offer is looked up again when the cached one is older than this.
const MAX_CHECK_AGE_SECONDS: i64 = 3600;

/// Where the update looks and writes.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// `owner/name` of the repository that publishes the releases.
    pub repo: String,
    /// The folder the panel leaves its request in (the root update service watches it).
    pub request_dir: PathBuf,
    /// The folder the update service writes its status to (root's, read only here).
    pub status_dir: PathBuf,
}

impl UpdateConfig {
    /// The defaults of an installed panel; `BUILTPANEL_UPDATE_REPO`, `BUILTPANEL_UPDATE_REQUEST_DIR` and
    /// `BUILTPANEL_UPDATE_STATUS_DIR` change them.
    #[must_use]
    pub fn from_env() -> Self {
        let var = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        let repo = var("BUILTPANEL_UPDATE_REPO", DEFAULT_REPO);
        Self {
            repo: if valid_repo(&repo) {
                repo
            } else {
                DEFAULT_REPO.to_owned()
            },
            request_dir: PathBuf::from(var(
                "BUILTPANEL_UPDATE_REQUEST_DIR",
                "/var/lib/builtpanel/update-request",
            )),
            status_dir: PathBuf::from(var(
                "BUILTPANEL_UPDATE_STATUS_DIR",
                "/var/lib/builtpanel-update",
            )),
        }
    }
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

/// What the update handlers share.
#[derive(Clone)]
pub struct UpdateState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// Where it looks and writes.
    pub config: Arc<UpdateConfig>,
    /// The checked settings schemas (the automatic update's settings are the system module's).
    pub settings: Arc<SettingsRegistry>,
}

impl FromRef<UpdateState> for PgPool {
    fn from_ref(s: &UpdateState) -> Self {
        s.db.clone()
    }
}

impl FromRef<UpdateState> for SessionStore {
    fn from_ref(s: &UpdateState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the admin router.
pub fn router() -> Router<UpdateState> {
    Router::new()
        .route("/api/v1/admin/update/status", get(status))
        .route("/api/v1/admin/update/check", post(check))
        .route("/api/v1/admin/update/apply", post(apply))
}

// ------------------------------------------------------------------- views

/// A release that is newer than the installed one.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdateOffer {
    /// The version, `X.Y.Z`.
    #[schema(example = "0.1.15")]
    pub version: String,
    /// When the release was made (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub release_date: String,
    /// Always `stable`.
    pub channel: String,
    /// The Ed25519 signature of the release manifest (the installation checks it before installing).
    pub ed25519_signature: String,
    /// The release key that signed it.
    pub key_id: String,
    /// SHA-256 of the release archive, when the release site shows it (else empty).
    pub checksum_sha256: String,
    /// What changed, one line each.
    pub changelog: Vec<String>,
    /// Database migrations the release brings (not listed ahead of time: empty).
    pub required_migrations: Vec<String>,
    /// A release that does not start is put back by the installation itself.
    pub can_rollback: bool,
}

/// What the update service is doing, or did last.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRun {
    /// `running`, `completed`, `rolled_back` or `failed`.
    pub state: String,
    /// `request`, `downloading`, `verifying`, `completed`, `rolled_back` or `failed`.
    pub step: String,
    /// The version it is about.
    pub version: String,
    /// The version it started from.
    pub from_version: String,
    /// What happened, in a sentence.
    pub message: String,
    /// When this was written (RFC 3339, UTC).
    pub updated_at: String,
}

/// One earlier update.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateHistoryItem {
    /// Id.
    pub id: String,
    /// The version before.
    pub from_version: String,
    /// The version it went to.
    pub to_version: String,
    /// Always `stable`.
    pub channel: String,
    /// `success`, `rolled_back` or `failed`.
    pub status: String,
    /// When it ended (RFC 3339, UTC).
    pub timestamp: String,
    /// What happened.
    pub details: String,
}

/// The update page's numbers.
#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateStatus {
    /// The version that runs.
    pub current_version: String,
    /// Always `stable`.
    pub channel: String,
    /// When the release site was last asked (RFC 3339, UTC); absent before the first check.
    pub last_checked_at: Option<String>,
    /// The newer release, if the last check found one.
    pub available_update: Option<UpdateOffer>,
    /// Always absent: a release that does not start is put back at once, there is no later rollback.
    pub previous_version: Option<String>,
    /// Always false (see `previous_version`).
    pub can_rollback: bool,
    /// This server has the update service (installed by the installer from the release that brought it);
    /// without it a new version is installed by running the installer again.
    pub can_apply: bool,
    /// What the update service is doing or did last.
    pub run: Option<UpdateRun>,
    /// The automatic update.
    pub auto: AutoUpdateView,
    /// Earlier updates, newest first.
    pub history: Vec<UpdateHistoryItem>,
}

/// The automatic update's settings, and when it will act on the release on offer.
#[derive(Debug, Serialize, ToSchema)]
pub struct AutoUpdateView {
    /// Switched on (setting `system.auto_update`; on unless an administrator switched it off).
    pub enabled: bool,
    /// Days a release waits after it appears (`system.auto_update_delay_days`).
    pub delay_days: i64,
    /// The hour, UTC, in which it starts (`system.auto_update_hour`); the window is six hours long.
    pub hour_utc: i64,
    /// When it will install the release on offer (RFC 3339, UTC); absent when it is off, nothing is on
    /// offer, this server cannot update itself, or the attempt has been made.
    pub scheduled_for: Option<String>,
}

/// The answer of a check.
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckResult {
    /// The release site answered.
    pub checked: bool,
    /// There is a newer release.
    pub available: bool,
    /// It.
    pub update: Option<UpdateOffer>,
}

/// What to update to.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyUpdate {
    /// The version shown by the last check; any other is refused.
    #[schema(example = "0.1.15")]
    pub version: String,
}

/// The update was handed to the update service.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApplyStarted {
    /// Always true.
    pub ok: bool,
    /// Always `started`: follow `run` in the status.
    pub status: String,
    /// The version it will install.
    pub version: String,
}

// ------------------------------------------------------------------- checking the release site

/// A version as numbers.
pub type Version = (u64, u64, u64);

/// `1.2.3` (a leading `v` is allowed) as numbers; anything else is `None`.
#[must_use]
pub fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// What the signed release manifest says, read without checking the signature (the installation checks
/// it before it installs anything; here it only tells what to offer).
#[derive(Debug, PartialEq, Eq)]
pub struct ManifestFacts {
    /// The release key's name.
    pub kid: String,
    /// The signature, as published.
    pub signature: String,
    /// The version.
    pub version: String,
    /// When it was made (Unix seconds).
    pub created_at: i64,
}

/// Reads the facts out of a `release-manifest.json`.
///
/// # Errors
/// A sentence when the text is not a manifest.
pub fn read_manifest(text: &str) -> Result<ManifestFacts, String> {
    #[derive(Deserialize)]
    struct Envelope {
        kid: String,
        signature: String,
        payload: String,
    }
    #[derive(Deserialize)]
    struct Payload {
        version: String,
        created_at: i64,
    }
    let bad = || "The release manifest is not in the expected form.".to_owned();
    let envelope: Envelope = serde_json::from_str(text).map_err(|_| bad())?;
    let raw = URL_SAFE_NO_PAD
        .decode(envelope.payload.trim_end_matches('='))
        .map_err(|_| bad())?;
    let payload: Payload = serde_json::from_slice(&raw).map_err(|_| bad())?;
    if parse_version(&payload.version).is_none() {
        return Err(bad());
    }
    Ok(ManifestFacts {
        kid: envelope.kid,
        signature: envelope.signature,
        version: payload.version,
        created_at: payload.created_at,
    })
}

/// The change lines of a release's `CHANGELOG.md` (the `- ` items), without the markdown emphasis.
#[must_use]
pub fn changelog_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_end().strip_prefix("- "))
        .map(|l| l.replace("**", ""))
        .filter(|l| !l.is_empty())
        .take(20)
        .map(|l| l.chars().take(200).collect())
        .collect()
}

fn get_text(agent: &ureq::Agent, url: &str, limit: u64) -> Result<String, String> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("The release site could not be reached ({e})."))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_string()
        .map_err(|e| format!("The release site's answer could not be read ({e})."))
}

/// Asks the release site for its newest release; `Ok(None)` when it is not newer than `current`.
fn fetch_offer(repo: &str, current: Version) -> Result<Option<UpdateOffer>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(format!("builtpanel/{CURRENT_VERSION}"))
        .build()
        .into();
    let base = format!("https://github.com/{repo}/releases");
    let manifest = read_manifest(&get_text(
        &agent,
        &format!("{base}/latest/download/release-manifest.json"),
        256 * 1024,
    )?)?;
    let newest = parse_version(&manifest.version).ok_or("The release manifest has no version.")?;
    if newest <= current {
        return Ok(None);
    }
    let v = &manifest.version;
    let changelog = get_text(
        &agent,
        &format!("{base}/download/v{v}/CHANGELOG.md"),
        128 * 1024,
    )
    .map(|t| changelog_lines(&t))
    .unwrap_or_default();
    let checksum = get_text(
        &agent,
        &format!("{base}/download/v{v}/builtpanel-{v}-linux-x86_64.tar.gz.sha256"),
        4096,
    )
    .ok()
    .and_then(|t| t.split_whitespace().next().map(str::to_owned))
    .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    .unwrap_or_default();
    Ok(Some(UpdateOffer {
        version: manifest.version.clone(),
        release_date: rfc3339(manifest.created_at),
        channel: "stable".to_owned(),
        ed25519_signature: manifest.signature,
        key_id: manifest.kid,
        checksum_sha256: checksum,
        changelog,
        required_migrations: Vec::new(),
        can_rollback: true,
    }))
}

/// The last check's answer, kept for the page (one panel per process).
#[derive(Clone)]
struct Checked {
    at: i64,
    offer: Option<UpdateOffer>,
}

static LAST_CHECK: Mutex<Option<Checked>> = Mutex::new(None);

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn cached() -> Option<Checked> {
    LAST_CHECK.lock().ok().and_then(|g| g.clone())
}

async fn check_now(config: &UpdateConfig) -> Result<Checked, AppError> {
    let repo = config.repo.clone();
    let current = parse_version(CURRENT_VERSION).unwrap_or((0, 0, 0));
    let offer = tokio::task::spawn_blocking(move || fetch_offer(&repo, current))
        .await
        .map_err(AppError::internal)?
        .map_err(|detail| AppError::unavailable("release_site_unavailable", detail))?;
    let checked = Checked { at: now(), offer };
    if let Ok(mut slot) = LAST_CHECK.lock() {
        *slot = Some(checked.clone());
    }
    Ok(checked)
}

async fn read_json<T: serde::de::DeserializeOwned>(path: PathBuf) -> Option<T> {
    let bytes = tokio::fs::read(path).await.ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

// ------------------------------------------------------------------- the request

/// Leaves the request for the update service: one version number, put in place whole. `false` when a
/// request is already waiting.
async fn request_update(config: &UpdateConfig, version: &str) -> std::io::Result<bool> {
    let request = config.request_dir.join("request");
    if tokio::fs::symlink_metadata(&request).await.is_ok() {
        return Ok(false);
    }
    let tmp = config.request_dir.join("request.tmp");
    tokio::fs::write(&tmp, format!("{version}\n")).await?;
    tokio::fs::rename(&tmp, &request).await?;
    Ok(true)
}

// ------------------------------------------------------------------- the automatic update

/// How long the automatic update may start after its hour (a panel that was down at that hour still
/// updates the same morning, not in the middle of the afternoon).
const WINDOW_SECONDS: i64 = 6 * 3600;
const DAY: i64 = 86_400;

/// The settings of the automatic update (the defaults when they cannot be read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AutoSettings {
    enabled: bool,
    delay_days: i64,
    hour: i64,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_days: 3,
            hour: 3,
        }
    }
}

async fn auto_settings(db: &PgPool, registry: &SettingsRegistry) -> AutoSettings {
    let defaults = AutoSettings::default();
    let Ok(mut conn) = db.acquire().await else {
        return defaults;
    };
    let Ok(values) = builtpanel_settings::get(registry, &mut conn, "system", Scope::Global).await
    else {
        return defaults;
    };
    AutoSettings {
        enabled: values
            .get("auto_update")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(defaults.enabled),
        delay_days: values
            .get("auto_update_delay_days")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(defaults.delay_days)
            .clamp(0, 30),
        hour: values
            .get("auto_update_hour")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(defaults.hour)
            .clamp(0, 23),
    }
}

/// Whether `t` (Unix seconds) is within the daily window that starts at `hour` UTC.
fn in_window(t: i64, hour: i64) -> bool {
    (t - hour * 3600).rem_euclid(DAY) < WINDOW_SECONDS
}

/// The first moment the automatic update may start for a release first seen at `first_seen`: after its
/// waiting days, within the daily window.
fn scheduled_at(first_seen: i64, delay_days: i64, hour: i64) -> i64 {
    let earliest = first_seen + delay_days * DAY;
    if in_window(earliest, hour) {
        return earliest;
    }
    let today = earliest.div_euclid(DAY) * DAY + hour * 3600;
    if today >= earliest {
        today
    } else {
        today + DAY
    }
}

/// What is known of a release that was on offer.
struct OfferRow {
    first_seen: i64,
    notified: bool,
    attempted: bool,
    failure_notified: bool,
}

async fn offer_row(db: &PgPool, version: &str) -> Option<OfferRow> {
    sqlx::query_as::<_, (i64, bool, bool, bool)>(
        "SELECT extract(epoch FROM first_seen_at)::bigint, notified_at IS NOT NULL,
                auto_attempted_at IS NOT NULL, failure_notified_at IS NOT NULL
           FROM panel_updates WHERE version = $1",
    )
    .bind(version)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(
        |(first_seen, notified, attempted, failure_notified)| OfferRow {
            first_seen,
            notified,
            attempted,
            failure_notified,
        },
    )
}

/// What an event being sent marks as done.
#[derive(Clone, Copy)]
enum Mark {
    Notified,
    Attempted,
    FailureNotified,
}

async fn publish(
    db: &PgPool,
    version: &str,
    stage: &str,
    auto_in_days: Option<i64>,
    mark: Mark,
) -> Result<(), String> {
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    builtpanel_events::publish(
        &mut tx,
        &Event::PanelUpdate {
            version: version.to_owned(),
            stage: stage.to_owned(),
            auto_in_days,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(match mark {
        Mark::Notified => "UPDATE panel_updates SET notified_at = now() WHERE version = $1",
        Mark::Attempted => "UPDATE panel_updates SET auto_attempted_at = now() WHERE version = $1",
        Mark::FailureNotified => {
            "UPDATE panel_updates SET failure_notified_at = now() WHERE version = $1"
        }
    })
    .bind(version)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}

/// One look at the release on offer: tell the administrators once, install it by itself when the
/// settings say so and its time has come, and tell them when such an update did not work.
///
/// # Errors
/// A sentence for the log.
pub async fn auto_round(
    db: &PgPool,
    registry: &SettingsRegistry,
    config: &UpdateConfig,
    now: i64,
    offer: Option<&UpdateOffer>,
) -> Result<(), String> {
    let Some(offer) = offer else {
        return Ok(());
    };
    let version = offer.version.as_str();
    sqlx::query("INSERT INTO panel_updates (version) VALUES ($1) ON CONFLICT (version) DO NOTHING")
        .bind(version)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    let row = offer_row(db, version)
        .await
        .ok_or("the release's row is missing")?;
    let auto = auto_settings(db, registry).await;
    let can_apply = config.request_dir.is_dir();
    if !row.notified {
        let in_days = (auto.enabled && can_apply).then(|| {
            let at = scheduled_at(row.first_seen, auto.delay_days, auto.hour);
            ((at - now).max(0) + DAY - 1) / DAY
        });
        publish(db, version, "available", in_days, Mark::Notified).await?;
    }
    let run: Option<UpdateRun> = read_json(config.status_dir.join("status.json")).await;
    let running = run
        .as_ref()
        .is_some_and(|r| r.state == "running" && updated_recently(&r.updated_at));
    if row.attempted {
        let failed = run.as_ref().is_some_and(|r| {
            r.version == version && matches!(r.state.as_str(), "failed" | "rolled_back")
        });
        if failed && !row.failure_notified {
            publish(db, version, "failed", None, Mark::FailureNotified).await?;
        }
        return Ok(());
    }
    if !auto.enabled
        || !can_apply
        || running
        || now < row.first_seen + auto.delay_days * DAY
        || !in_window(now, auto.hour)
    {
        return Ok(());
    }
    if !request_update(config, version)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    builtpanel_audit::insert(
        db,
        &AuditEntry::new("panel.update.automatic", Outcome::Success)
            .target("panel", version)
            .metadata(serde_json::json!({ "from": CURRENT_VERSION, "to": version })),
    )
    .await
    .map_err(|e| e.to_string())?;
    publish(db, version, "started", None, Mark::Attempted).await
}

/// Looks for a new release now and then (the first time two minutes after the start, then every half
/// hour; the release site is asked at most every six hours) and runs [`auto_round`].
pub async fn run_auto(db: PgPool, registry: Arc<SettingsRegistry>, config: Arc<UpdateConfig>) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    let mut every = tokio::time::interval(Duration::from_secs(1800));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        let fresh = cached().filter(|c| now() - c.at < 6 * 3600);
        let checked = match fresh {
            Some(c) => Some(c),
            None => match check_now(&config).await {
                Ok(c) => Some(c),
                Err(error) => {
                    tracing::debug!(code = error.code(), "the release site was not asked");
                    cached()
                }
            },
        };
        let offer = checked.and_then(|c| c.offer);
        if let Err(error) = auto_round(&db, &registry, &config, now(), offer.as_ref()).await {
            tracing::warn!(%error, "the automatic update check failed");
        }
    }
}

// ------------------------------------------------------------------- handlers

/// The update page's numbers.
#[utoipa::path(
    get,
    path = "/api/v1/admin/update/status",
    tag = "system",
    operation_id = "updateStatus",
    security(("session" = [])),
    responses(
        (status = 200, description = "What runs, what is on offer, what the update service did", body = UpdateStatus),
        (status = 403, description = "`permission_denied` (needs admin.settings.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn status(
    State(state): State<UpdateState>,
    _who: Require<SettingsRead>,
) -> Result<Json<UpdateStatus>, AppError> {
    let dir = &state.config.status_dir;
    let history: Vec<UpdateHistoryItem> = read_json(dir.join("history.json"))
        .await
        .unwrap_or_default();
    let run: Option<UpdateRun> = read_json(dir.join("status.json")).await;
    let checked = cached();
    let installed = parse_version(CURRENT_VERSION).unwrap_or((0, 0, 0));
    // An offer that the installed version has caught up with (after an update) is gone.
    let offer = checked
        .as_ref()
        .and_then(|c| c.offer.clone())
        .filter(|o| parse_version(&o.version).is_some_and(|v| v > installed));
    let can_apply = state.config.request_dir.is_dir();
    let auto = auto_settings(&state.db, &state.settings).await;
    let mut scheduled_for = None;
    if let (true, true, Some(o)) = (auto.enabled, can_apply, offer.as_ref())
        && let Some(row) = offer_row(&state.db, &o.version).await
        && !row.attempted
    {
        scheduled_for = Some(rfc3339(scheduled_at(
            row.first_seen,
            auto.delay_days,
            auto.hour,
        )));
    }
    Ok(Json(UpdateStatus {
        current_version: CURRENT_VERSION.to_owned(),
        channel: "stable".to_owned(),
        last_checked_at: checked.as_ref().map(|c| rfc3339(c.at)),
        available_update: offer,
        previous_version: None,
        can_rollback: false,
        can_apply,
        run,
        auto: AutoUpdateView {
            enabled: auto.enabled,
            delay_days: auto.delay_days,
            hour_utc: auto.hour,
            scheduled_for,
        },
        history,
    }))
}

/// Ask the release site for its newest release.
#[utoipa::path(
    post,
    path = "/api/v1/admin/update/check",
    tag = "system",
    operation_id = "updateCheck",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Checked", body = CheckResult),
        (status = 403, description = "`permission_denied` (needs admin.settings.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`release_site_unavailable`: the release site could not be reached", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn check(
    State(state): State<UpdateState>,
    _who: Require<SettingsManage>,
) -> Result<Json<CheckResult>, AppError> {
    let checked = check_now(&state.config).await?;
    Ok(Json(CheckResult {
        checked: true,
        available: checked.offer.is_some(),
        update: checked.offer,
    }))
}

/// Update the panel to the version the last check found: the request goes to the update service,
/// which installs it (follow `run` in the status).
#[utoipa::path(
    post,
    path = "/api/v1/admin/update/apply",
    tag = "system",
    operation_id = "updateApply",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ApplyUpdate,
    responses(
        (status = 202, description = "Handed to the update service", body = ApplyStarted),
        (status = 403, description = "`permission_denied` (needs admin.settings.manage), `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`update_not_available` (this server has no update service), `no_such_update` (not the version on offer) or `update_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`release_site_unavailable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn apply(
    State(state): State<UpdateState>,
    RequireRecent(who, _): RequireRecent<SettingsManage>,
    Json(body): Json<ApplyUpdate>,
) -> Result<(axum::http::StatusCode, Json<ApplyStarted>), AppError> {
    if !state.config.request_dir.is_dir() {
        return Err(AppError::conflict(
            "update_not_available",
            "This server was installed before the update service existed: run the installer once more (it keeps your data) to get it.",
        ));
    }
    let wanted = body.version.trim().trim_start_matches('v').to_owned();
    // Only what the release site offers, and looked at a moment ago.
    let checked = match cached().filter(|c| now() - c.at < MAX_CHECK_AGE_SECONDS) {
        Some(c) => c,
        None => check_now(&state.config).await?,
    };
    let offer = checked
        .offer
        .filter(|o| o.version == wanted)
        .ok_or_else(|| {
            AppError::conflict(
                "no_such_update",
                "That is not the version on offer: check for updates again.",
            )
        })?;
    if let Some(run) = read_json::<UpdateRun>(state.config.status_dir.join("status.json")).await
        && run.state == "running"
        && updated_recently(&run.updated_at)
    {
        return Err(AppError::conflict(
            "update_running",
            "An update is already running.",
        ));
    }
    if !request_update(&state.config, &offer.version)
        .await
        .map_err(AppError::internal)?
    {
        return Err(AppError::conflict(
            "update_running",
            "An update is already waiting to start.",
        ));
    }
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("panel.update.requested", Outcome::Success)
            .actor(who.account_id())
            .target("panel", &offer.version)
            .metadata(serde_json::json!({ "from": CURRENT_VERSION, "to": offer.version })),
    )
    .await
    .map_err(AppError::internal)?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(ApplyStarted {
            ok: true,
            status: "started".to_owned(),
            version: offer.version,
        }),
    ))
}

/// A run that reported itself within the last half hour is still going (a crashed one is not
/// held against a new request forever).
fn updated_recently(updated_at: &str) -> bool {
    // `YYYY-MM-DDTHH:MM:SSZ`
    let b = updated_at.as_bytes();
    if b.len() != 20 || b[10] != b'T' || b[19] != b'Z' {
        return false;
    }
    let n = |r: std::ops::Range<usize>| updated_at[r].parse::<i64>().ok();
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) =
        (n(0..4), n(5..7), n(8..10), n(11..13), n(14..16), n(17..19))
    else {
        return false;
    };
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let at = days * 86_400 + h * 3600 + mi * 60 + s;
    (now() - at) < 1800
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_three_plain_numbers() {
        assert_eq!(parse_version("0.1.14"), Some((0, 1, 14)));
        assert_eq!(parse_version("v1.20.3\n"), Some((1, 20, 3)));
        for bad in [
            "",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-beta",
            "../1.2.3",
            "1.2.-3",
            "1..3",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert!(parse_version("0.1.15") > parse_version("0.1.14"));
        assert!(parse_version("0.2.0") > parse_version("0.1.99"));
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn the_manifest_gives_version_date_and_key() {
        let payload = URL_SAFE_NO_PAD.encode(
            br#"{"commit":"abc","components":[],"created_at":1795350356,"v":1,"version":"0.1.14"}"#,
        );
        let text = format!(
            r#"{{"kid":"release-1","alg":"EdDSA","payload":"{payload}","signature":"c2ln"}}"#
        );
        assert_eq!(
            read_manifest(&text).unwrap(),
            ManifestFacts {
                kid: "release-1".into(),
                signature: "c2ln".into(),
                version: "0.1.14".into(),
                created_at: 1_795_350_356,
            }
        );
        for bad in ["", "{}", r#"{"kid":"a","signature":"b","payload":"!!"}"#] {
            assert!(read_manifest(bad).is_err(), "{bad:?}");
        }
        let hostile = URL_SAFE_NO_PAD.encode(br#"{"version":"1.2.3; rm -rf /","created_at":1}"#);
        let text = format!(r#"{{"kid":"a","signature":"b","payload":"{hostile}"}}"#);
        assert!(read_manifest(&text).is_err(), "only X.Y.Z is a version");
    }

    #[test]
    fn the_changelog_gives_its_items() {
        let text = "# Built Panel 0.1.14\n\nChanges:\n\n## New\n- block addresses\n- **web**: containers tab\n  - not an item of its own\n\n## Fixes\n- one more\n";
        assert_eq!(
            changelog_lines(text),
            ["block addresses", "web: containers tab", "one more"]
        );
    }

    #[test]
    fn the_automatic_update_waits_its_days_and_starts_in_its_hour() {
        let day = |d: i64, h: i64| d * DAY + h * 3600;
        // Seen at 14:00 on day 10, three days, hour 3: day 13 at 14:00 is outside the window, so day 14, 03:00.
        assert_eq!(scheduled_at(day(10, 14), 3, 3), day(14, 3));
        // Seen at 02:00: the waiting ends at 02:00 on day 13, before the hour: that day's 03:00.
        assert_eq!(scheduled_at(day(10, 2), 3, 3), day(13, 3));
        // No waiting, seen inside the window: at once.
        assert_eq!(scheduled_at(day(10, 4), 0, 3), day(10, 4));
        // No waiting, seen after the window: the next day's hour.
        assert_eq!(scheduled_at(day(10, 20), 0, 3), day(11, 3));
        // The window wraps over midnight.
        assert!(in_window(day(5, 23), 22) && in_window(day(6, 3), 22) && !in_window(day(6, 5), 22));
        assert!(
            !in_window(day(5, 2), 3)
                && in_window(day(5, 3), 3)
                && in_window(day(5, 8), 3)
                && !in_window(day(5, 9), 3)
        );
    }

    #[test]
    fn repositories_are_owner_slash_name() {
        assert!(valid_repo("khairulalamruet-ai/DPanel"));
        for bad in ["", "a", "a/b/c", "a/../b", "a/b c", "a/b?x=1", "/b", "a/"] {
            assert!(!valid_repo(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_status_time_is_recent_for_half_an_hour() {
        let at = |secs_ago: i64| {
            let t = now() - secs_ago;
            // civil-from-days, the inverse of `updated_recently`'s arithmetic
            let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            format!(
                "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
                rem / 3600,
                rem % 3600 / 60,
                rem % 60
            )
        };
        assert!(updated_recently(&at(5)));
        assert!(!updated_recently(&at(3600)));
        assert!(!updated_recently("yesterday"));
    }
}
