//! Releases and the daily integrity scan (T17.14, T17.16).
//!
//! The release pipeline signs a manifest of the release's files with the release key; the
//! authority checks that signature when the manifest is published and keeps the file hashes.
//! Every day an installation asks for a challenge, hashes the files it runs, and sends the
//! hashes back signed with its identity key. The authority compares them with the release:
//! a file that is not what the release says is a `failed` scan and an integrity event.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use builtpanel_http_kit::{AppError, FieldError};
use builtpanel_license::{PublicKeys, SignedManifest, verify_with};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    Admin, LaState, audit,
    public::{Proof, proof_message, prove_request},
};

/// How long a scan may be answered after its challenge.
const SCAN_TTL_SECS: i64 = 600;

/// The routes.
pub fn router() -> Router<LaState> {
    Router::new()
        .route("/api/v1/admin/release-keys", post(add_release_key))
        .route("/api/v1/admin/releases", get(list_releases).post(publish))
        .route("/api/v1/admin/integrity/events", get(list_events))
        .route("/api/v1/admin/recovery-requests", get(list_recoveries))
        .route(
            "/api/v1/admin/recovery-requests/{id}/approve",
            post(approve_recovery),
        )
        .route(
            "/api/v1/admin/recovery-requests/{id}/reject",
            post(reject_recovery),
        )
        .route(
            "/api/v1/admin/installations/{id}/integrity/dismiss",
            post(dismiss_integrity),
        )
        .route("/api/v1/releases/latest", get(latest))
        .route("/api/v1/integrity/challenge", post(challenge))
        .route("/api/v1/integrity/report", post(report))
}

fn bad(field: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, "invalid", message)])
}

fn db(e: sqlx::Error) -> AppError {
    AppError::internal(e)
}

/// A key of the release pipeline (the public half only).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewReleaseKey {
    /// Its id (as written into the manifests it signs).
    pub kid: String,
    /// The public key, base64 (32 bytes).
    pub public_key: String,
}

async fn add_release_key(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewReleaseKey>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    if !(1..=64).contains(&req.kid.len())
        || !req
            .kid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    {
        return Err(bad("kid", "1 to 64 letters, digits, - and _."));
    }
    let public: Vec<u8> = STANDARD
        .decode(req.public_key.trim())
        .ok()
        .filter(|k| k.len() == 32)
        .ok_or_else(|| bad("public_key", "A base64 Ed25519 public key (32 bytes)."))?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM signing_keys WHERE kid = $1)")
            .bind(&req.kid)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if taken {
        return Err(AppError::conflict(
            "key_id_taken",
            "This key id is used already.",
        ));
    }
    // The new key becomes the active one; manifests the old one signed stay valid.
    sqlx::query("UPDATE signing_keys SET status = 'retired', retired_at = now() WHERE purpose = 'release' AND status = 'active'")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO signing_keys (kid, purpose, public_key) VALUES ($1, 'release', $2)")
        .bind(&req.kid)
        .bind(&public)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    audit(
        &mut *tx,
        &admin.email,
        "release_key.added",
        Some(&req.kid),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::CREATED)
}

/// A release to publish.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewRelease {
    /// `stable` (the default) or `beta`.
    #[serde(default)]
    pub channel: Option<String>,
    /// What `bpctl release sign` wrote.
    pub manifest: SignedManifest,
}

/// A published release.
#[derive(Serialize, FromRow)]
pub struct ReleaseInfo {
    /// Its version.
    pub version: String,
    /// `stable` or `beta`.
    pub channel: String,
    /// The key that signed it.
    pub kid: String,
    /// How many files it lists.
    pub files: i64,
    /// Published (Unix seconds).
    pub created_at: i64,
}

async fn release_keys(state: &LaState) -> Result<PublicKeys, AppError> {
    let rows: Vec<(String, Vec<u8>)> =
        sqlx::query_as("SELECT kid, public_key FROM signing_keys WHERE purpose = 'release'")
            .fetch_all(&state.db)
            .await
            .map_err(db)?;
    let mut keys = PublicKeys::new();
    for (kid, key) in rows {
        let key: [u8; 32] = key
            .try_into()
            .map_err(|_| AppError::internal("bad release key"))?;
        keys = keys.with(&kid, key).map_err(AppError::internal)?;
    }
    Ok(keys)
}

async fn publish(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewRelease>,
) -> Result<(StatusCode, Json<ReleaseInfo>), AppError> {
    admin.may_change()?;
    let channel = req.channel.as_deref().unwrap_or("stable");
    if !matches!(channel, "stable" | "beta") {
        return Err(bad("channel", "stable or beta."));
    }
    // The signature must be the release key's; a manifest nobody signed is not published.
    let manifest = req
        .manifest
        .verify(&release_keys(&state).await?)
        .map_err(|e| {
            AppError::validation(vec![FieldError::new(
                "manifest",
                "not_valid",
                e.to_string(),
            )])
        })?;
    for c in &manifest.components {
        if !c
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
            || c.name.len() < 2
            || c.name.len() > 41
            || !c
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_".contains(c))
        {
            return Err(bad(
                "manifest",
                "A file name is not usable for a release component.",
            ));
        }
    }
    let document = serde_json::to_value(&req.manifest).map_err(AppError::internal)?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let id: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO release_manifests (version, channel, document, kid) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&manifest.version)
    .bind(channel)
    .bind(&document)
    .bind(&req.manifest.kid)
    .fetch_one(&mut *tx)
    .await;
    let id = id.map_err(|e| {
        if e.as_database_error()
            .and_then(|d| d.code().map(|c| c.to_string()))
            .as_deref()
            == Some("23505")
        {
            AppError::conflict("release_exists", "This version is published already.")
        } else {
            db(e)
        }
    })?;
    for c in &manifest.components {
        let hash = (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&c.sha256[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(AppError::internal)?;
        sqlx::query("INSERT INTO release_components (manifest_id, name, sha256, size_bytes) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(&c.name)
            .bind(hash)
            .bind(i64::try_from(c.size).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    audit(
        &mut *tx,
        &admin.email,
        "release.published",
        Some(&manifest.version),
        serde_json::json!({"channel": channel, "files": manifest.components.len()}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok((
        StatusCode::CREATED,
        Json(ReleaseInfo {
            version: manifest.version,
            channel: channel.to_owned(),
            kid: req.manifest.kid,
            files: i64::try_from(manifest.components.len()).unwrap_or(0),
            created_at: 0,
        }),
    ))
}

async fn list_releases(
    State(state): State<LaState>,
    _a: Admin,
) -> Result<Json<Vec<ReleaseInfo>>, AppError> {
    sqlx::query_as(
        "SELECT m.version, m.channel, m.kid,
                (SELECT count(*) FROM release_components c WHERE c.manifest_id = m.id) AS files,
                extract(epoch FROM m.created_at)::bigint AS created_at
         FROM release_manifests m ORDER BY m.created_at DESC LIMIT 200",
    )
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

/// Which release.
#[derive(Deserialize)]
pub struct WhichRelease {
    /// `stable` (the default) or `beta`.
    pub channel: Option<String>,
    /// A particular version instead of the newest.
    pub version: Option<String>,
}

/// The newest manifest of a channel (or a given version), as it was signed: the installation
/// checks the signature itself.
async fn latest(
    State(state): State<LaState>,
    Query(q): Query<WhichRelease>,
) -> Result<Json<serde_json::Value>, AppError> {
    let document: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT document FROM release_manifests
         WHERE ($1::text IS NULL OR version = $1) AND channel = COALESCE($2, 'stable')
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(q.version)
    .bind(q.channel)
    .fetch_optional(&state.db)
    .await
    .map_err(db)?;
    document
        .map(Json)
        .ok_or_else(|| AppError::not_found("release"))
}

/// What a scan is asked about.
#[derive(Deserialize)]
pub struct ChallengeRequest {
    /// The proof (host facts included).
    #[serde(flatten)]
    pub proof: Proof,
    /// The release version the installation runs.
    pub version: String,
}

/// A scan to answer.
#[derive(Serialize)]
pub struct ScanChallenge {
    /// This scan.
    pub scan_id: Uuid,
    /// What the report is signed with (base64url).
    pub nonce: String,
    /// Until when it may be answered (Unix seconds).
    pub expires_at: i64,
    /// The files to hash (the release's component names); empty for a release not known.
    pub components: Vec<String>,
}

async fn challenge(
    State(state): State<LaState>,
    Json(req): Json<ChallengeRequest>,
) -> Result<Json<ScanChallenge>, AppError> {
    if req.version.is_empty() || req.version.len() > 64 {
        return Err(bad("version", "1 to 64 characters."));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    prove_request(&state, &mut tx, "integrity", &req.proof).await?;
    let mut nonce = [0u8; 32];
    {
        use std::io::Read as _;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut nonce))
            .map_err(AppError::internal)?;
    }
    let (scan_id, expires_at): (Uuid, i64) = sqlx::query_as(
        "INSERT INTO integrity_scans (installation_id, nonce, expires_at, version)
         VALUES ($1, $2, now() + make_interval(secs => $3::double precision), $4)
         RETURNING id, extract(epoch FROM expires_at)::bigint",
    )
    .bind(req.proof.installation_id)
    .bind(nonce.to_vec())
    .bind(SCAN_TTL_SECS)
    .bind(&req.version)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let components: Vec<String> = sqlx::query_scalar(
        "SELECT c.name FROM release_components c JOIN release_manifests m ON m.id = c.manifest_id
         WHERE m.version = $1 ORDER BY c.name",
    )
    .bind(&req.version)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(ScanChallenge {
        scan_id,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        expires_at,
        components,
    }))
}

/// The hashes an installation found.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportRequest {
    /// The installation.
    pub installation_id: Uuid,
    /// The scan.
    pub scan_id: Uuid,
    /// File name to the SHA-256 (hex) of what is installed; a file that is not there is left out.
    pub files: BTreeMap<String, String>,
    /// The signature of [`report_text`] under the scan's nonce (base64url).
    pub signature: String,
}

/// The text a report is signed over.
#[must_use]
pub fn report_text(scan_id: Uuid, files: &BTreeMap<String, String>) -> String {
    builtpanel_license::integrity_report_text(&scan_id.to_string(), files)
}

/// How a scan came out.
#[derive(Serialize)]
pub struct ScanResult {
    /// `passed`, `failed` (a file is not the release's), `partial` (a file is missing) or
    /// `unverifiable` (the release is not known).
    pub result: String,
    /// The files that are not what the release says.
    pub mismatched: Vec<String>,
    /// The files that were not reported.
    pub missing: Vec<String>,
    /// What the installation's state is now: `ok`, `failed` or `recovering`.
    pub integrity: String,
}

async fn report(
    State(state): State<LaState>,
    Json(req): Json<ReportRequest>,
) -> Result<Json<ScanResult>, AppError> {
    if req.files.len() > 256
        || req.files.iter().any(|(n, h)| {
            n.len() > 64 || h.len() != 64 || !h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        })
    {
        return Err(bad(
            "files",
            "At most 256 files, each with a lower-case hex SHA-256.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    let scan: Option<(Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT nonce, version FROM integrity_scans
         WHERE id = $1 AND installation_id = $2 AND reported_at IS NULL AND expires_at > now()
         FOR UPDATE",
    )
    .bind(req.scan_id)
    .bind(req.installation_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let (nonce, version) = scan.ok_or_else(|| {
        AppError::forbidden(
            "scan_not_open",
            "This scan is not open (it was answered, or it expired).",
        )
    })?;
    let public: Vec<u8> = sqlx::query_scalar("SELECT public_key FROM installations WHERE id = $1")
        .bind(req.installation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
    let signature = URL_SAFE_NO_PAD
        .decode(&req.signature)
        .map_err(|_| bad("signature", "base64url."))?;
    let message = proof_message(
        "integrity-report",
        &URL_SAFE_NO_PAD.encode(&nonce),
        req.installation_id,
        Some(&report_text(req.scan_id, &req.files)),
    );
    if !verify_with(&public, &message, &signature) {
        return Err(AppError::authentication_failed(
            "bad_signature",
            "The report is not signed by this installation.",
        ));
    }
    // What the release says.
    let wanted: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT c.name, c.sha256 FROM release_components c JOIN release_manifests m ON m.id = c.manifest_id
         WHERE m.version = $1",
    )
    .bind(version.as_deref())
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let (mut mismatched, mut missing) = (Vec::new(), Vec::new());
    for (name, hash) in &wanted {
        match req.files.get(name) {
            None => missing.push(name.clone()),
            Some(got) if *got != hex(hash) => mismatched.push(name.clone()),
            Some(_) => {}
        }
    }
    let result = if wanted.is_empty() {
        "unverifiable"
    } else if !mismatched.is_empty() {
        "failed"
    } else if !missing.is_empty() {
        "partial"
    } else {
        "passed"
    };
    let detail =
        serde_json::json!({"version": version, "mismatched": mismatched, "missing": missing});
    sqlx::query(
        "UPDATE integrity_scans SET reported_at = now(), result = $2, report = $3 WHERE id = $1",
    )
    .bind(req.scan_id)
    .bind(result)
    .bind(serde_json::json!({"files": req.files, "detail": detail}))
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    if result != "passed" {
        let kind = match result {
            "failed" => "mismatch",
            _ => "unverifiable",
        };
        sqlx::query("INSERT INTO integrity_events (installation_id, scan_id, kind, detail) VALUES ($1, $2, $3, $4)")
            .bind(req.installation_id)
            .bind(req.scan_id)
            .bind(kind)
            .bind(&detail)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    // What the result does to the installation. A changed or missing program makes it
    // `failed` (its license is refused until an administrator has looked); a clean scan only
    // ends a recovery an administrator approved. An unknown release changes nothing.
    let before: String =
        sqlx::query_scalar("SELECT integrity_state FROM installations WHERE id = $1 FOR UPDATE")
            .bind(req.installation_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let after = match (result, before.as_str()) {
        ("failed" | "partial", _) => "failed",
        ("passed", "recovering") => "ok",
        (_, current) => current,
    };
    if after != before {
        sqlx::query("UPDATE installations SET integrity_state = $2, integrity_changed_at = now() WHERE id = $1")
            .bind(req.installation_id)
            .bind(after)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let (kind, action) = if after == "ok" {
            ("recovered", "integrity.recovered")
        } else {
            ("note", "integrity.failed")
        };
        if after == "ok" {
            sqlx::query("INSERT INTO integrity_events (installation_id, scan_id, kind, detail) VALUES ($1, $2, $3, $4)")
                .bind(req.installation_id)
                .bind(req.scan_id)
                .bind(kind)
                .bind(serde_json::json!({"after": "clean scan"}))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            sqlx::query("UPDATE recovery_requests SET state = 'completed', updated_at = now() WHERE installation_id = $1 AND state = 'approved'")
                .bind(req.installation_id)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        audit(
            &mut *tx,
            "installation",
            action,
            Some(&req.installation_id.to_string()),
            detail.clone(),
        )
        .await?;
    }
    tx.commit().await.map_err(db)?;
    Ok(Json(ScanResult {
        result: result.to_owned(),
        mismatched,
        missing,
        integrity: after.to_owned(),
    }))
}

/// What happened to an installation's files.
#[derive(Serialize, FromRow)]
pub struct EventInfo {
    /// Id.
    pub id: Uuid,
    /// The installation.
    pub installation_id: Uuid,
    /// `mismatch`, `stale`, `unverifiable`, `recovered`, `note` or `binding`.
    pub kind: String,
    /// What was found.
    pub detail: serde_json::Value,
    /// When (Unix seconds).
    pub created_at: i64,
}

/// Which events.
#[derive(Deserialize)]
pub struct EventQuery {
    /// Only this installation's.
    pub installation_id: Option<Uuid>,
}

async fn list_events(
    State(state): State<LaState>,
    _a: Admin,
    Query(q): Query<EventQuery>,
) -> Result<Json<Vec<EventInfo>>, AppError> {
    sqlx::query_as(
        "SELECT id, installation_id, kind, detail, extract(epoch FROM created_at)::bigint AS created_at
         FROM integrity_events WHERE ($1::uuid IS NULL OR installation_id = $1)
         ORDER BY created_at DESC LIMIT 500",
    )
    .bind(q.installation_id)
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

/// A request for help after a failed check or a lost machine.
#[derive(Serialize, FromRow)]
pub struct RecoveryInfo {
    /// Id.
    pub id: Uuid,
    /// The installation.
    pub installation_id: Uuid,
    /// `requested`, `approved`, `rejected` or `completed`.
    pub state: String,
    /// What the installation wrote.
    pub note: Option<String>,
    /// The installation's integrity state now.
    pub integrity_state: String,
    /// Asked (Unix seconds).
    pub created_at: i64,
}

/// Which requests.
#[derive(Deserialize)]
pub struct RecoveryQuery {
    /// Only this state.
    pub state: Option<String>,
}

async fn list_recoveries(
    State(state): State<LaState>,
    _a: Admin,
    Query(q): Query<RecoveryQuery>,
) -> Result<Json<Vec<RecoveryInfo>>, AppError> {
    sqlx::query_as(
        "SELECT r.id, r.installation_id, r.state, r.note, i.integrity_state,
                extract(epoch FROM r.created_at)::bigint AS created_at
         FROM recovery_requests r JOIN installations i ON i.id = r.installation_id
         WHERE ($1::text IS NULL OR r.state = $1)
         ORDER BY r.created_at DESC LIMIT 500",
    )
    .bind(q.state)
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

/// What an administrator says about a decision.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// Why (kept in the audit log and in the event).
    #[serde(default)]
    pub note: Option<String>,
}

fn check_note(note: Option<&str>) -> Result<(), AppError> {
    if note.is_some_and(|n| n.chars().count() > 500) {
        return Err(bad("note", "At most 500 characters."));
    }
    Ok(())
}

async fn approve_recovery(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<Decision>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    check_note(req.note.as_deref())?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let installation: Option<Uuid> = sqlx::query_scalar(
        "UPDATE recovery_requests SET state = 'approved', updated_at = now()
         WHERE id = $1 AND state = 'requested' RETURNING installation_id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let installation = installation.ok_or_else(|| {
        AppError::conflict(
            "wrong_state",
            "Only a request that is waiting can be approved.",
        )
    })?;
    // The installation may recover: its next clean scan puts it right. (One that is not
    // failed has nothing to recover from; the request is only answered.)
    sqlx::query("UPDATE installations SET integrity_state = 'recovering', integrity_changed_at = now() WHERE id = $1 AND integrity_state = 'failed'")
        .bind(installation)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO integrity_events (installation_id, kind, detail) VALUES ($1, 'note', $2)",
    )
    .bind(installation)
    .bind(serde_json::json!({"recovery": "approved", "by": admin.email, "note": req.note}))
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    audit(
        &mut *tx,
        &admin.email,
        "recovery.approved",
        Some(&id.to_string()),
        serde_json::json!({"installation_id": installation, "note": req.note}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_recovery(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<Decision>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    check_note(req.note.as_deref())?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let installation: Option<Uuid> = sqlx::query_scalar(
        "UPDATE recovery_requests SET state = 'rejected', updated_at = now()
         WHERE id = $1 AND state = 'requested' RETURNING installation_id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let installation = installation.ok_or_else(|| {
        AppError::conflict(
            "wrong_state",
            "Only a request that is waiting can be rejected.",
        )
    })?;
    audit(
        &mut *tx,
        &admin.email,
        "recovery.rejected",
        Some(&id.to_string()),
        serde_json::json!({"installation_id": installation, "note": req.note}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// An administrator looked at a failed scan and found nothing wrong (a false alarm): the
/// installation is `ok` again at once.
async fn dismiss_integrity(
    State(state): State<LaState>,
    admin: Admin,
    Path(installation_id): Path<Uuid>,
    Json(req): Json<Decision>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    check_note(req.note.as_deref())?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let changed = sqlx::query("UPDATE installations SET integrity_state = 'ok', integrity_changed_at = now() WHERE id = $1 AND integrity_state <> 'ok'")
        .bind(installation_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::conflict(
            "wrong_state",
            "This installation has nothing to dismiss.",
        ));
    }
    sqlx::query("UPDATE recovery_requests SET state = 'completed', updated_at = now() WHERE installation_id = $1 AND state IN ('requested', 'approved')")
        .bind(installation_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO integrity_events (installation_id, kind, detail) VALUES ($1, 'note', $2)",
    )
    .bind(installation_id)
    .bind(serde_json::json!({"dismissed": true, "by": admin.email, "note": req.note}))
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    audit(
        &mut *tx,
        &admin.email,
        "integrity.dismissed",
        Some(&installation_id.to_string()),
        serde_json::json!({"note": req.note}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}
