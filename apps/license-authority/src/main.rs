//! The license authority's program: `builtpanel-license-authority` serves;
//! `builtpanel-license-authority create-admin <email> [admin|viewer]` makes an administrator
//! (the password is read from `BUILTPANEL_LA_ADMIN_PASSWORD`, never from the command line).

use std::{process::ExitCode, sync::Arc, time::Duration};

use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_license::{FileSigningKey, SigningKey as _};
use builtpanel_license_authority::{
    LaState, Settings, config::LaConfig, migrate, register_key, router,
};
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::postgres::PgPoolOptions;

const SERVICE: &str = "license-authority";
const EXIT_CONFIG: u8 = 78;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match LaConfig::load() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{SERVICE}: {err}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let mut telemetry = builtpanel_observability::TelemetryConfig::new(SERVICE);
    telemetry.log_format = if config.log_format == "pretty" {
        builtpanel_observability::LogFormat::Pretty
    } else {
        builtpanel_observability::LogFormat::Json
    };
    telemetry.log_filter.clone_from(&config.log_filter);
    let _telemetry = match builtpanel_observability::init(&telemetry) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("{SERVICE}: telemetry: {err}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let db = match PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(config.database_url.expose_secret())
        .await
    {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(error = %err, "cannot connect to the database");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = migrate(&db).await {
        tracing::error!(error = %err, "the migrations failed");
        return ExitCode::FAILURE;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("create-admin") {
        return create_admin(&db, &args[1..]).await;
    }
    let key = match FileSigningKey::load(&config.signing_key_file, &config.signing_kid) {
        Ok(k) => k,
        Err(err) => {
            tracing::error!(error = %err, file = %config.signing_key_file.display(), "cannot use the signing key");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    if let Err(err) = register_key(&db, &key).await {
        tracing::error!(error = %err.code(), "cannot register the signing key");
        return ExitCode::FAILURE;
    }
    tracing::info!(kid = key.kid(), "signing key ready");
    let hasher = match PasswordHasher::new(Argon2Params::default()) {
        Ok(h) => Arc::new(h),
        Err(err) => {
            tracing::error!(error = %err, "password hashing");
            return ExitCode::FAILURE;
        }
    };
    let state = LaState {
        db,
        signer: Arc::new(std::sync::RwLock::new(Arc::new(key))),
        settings: Settings::from_config(&config),
        hasher,
        prover: builtpanel_license_authority::domain_proof::default_prover(),
    };
    tokio::spawn(builtpanel_license_authority::billing::run_forever(
        state.clone(),
    ));
    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(error = %err, bind = %config.bind, "cannot listen");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(bind = %config.bind, "license authority listening");
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    if let Err(err) = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    {
        tracing::error!(error = %err, "server error");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn create_admin(db: &sqlx::PgPool, args: &[String]) -> ExitCode {
    let (Some(email), role) = (args.first(), args.get(1).map_or("admin", String::as_str)) else {
        eprintln!("usage: builtpanel-license-authority create-admin <email> [admin|viewer]");
        return ExitCode::from(64);
    };
    let Ok(password) = std::env::var("BUILTPANEL_LA_ADMIN_PASSWORD") else {
        eprintln!("set BUILTPANEL_LA_ADMIN_PASSWORD (at least 14 characters)");
        return ExitCode::from(64);
    };
    if password.chars().count() < 14 || !["admin", "viewer"].contains(&role) {
        eprintln!("the password needs 14 characters; the role is admin or viewer");
        return ExitCode::from(64);
    }
    let hasher = match PasswordHasher::new(Argon2Params::default()) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let Ok(hash) = hasher.hash(&SecretString::from(password)) else {
        return ExitCode::FAILURE;
    };
    match sqlx::query("INSERT INTO admin_users (email, password_hash, role) VALUES ($1, $2, $3)")
        .bind(email)
        .bind(hash)
        .bind(role)
        .execute(db)
        .await
    {
        Ok(_) => {
            println!("administrator {email} ({role}) created");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cannot create the administrator: {err}");
            ExitCode::FAILURE
        }
    }
}
