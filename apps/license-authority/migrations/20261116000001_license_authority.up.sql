-- The license authority's own database (T17.02): customers, what they bought, the licenses
-- issued, the installations that activated them, and what the authority signed. The private
-- signing keys are NOT here: signing_keys holds public halves only (see signing_keys.purpose).
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE customers (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text        NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    email      text        NOT NULL CHECK (length(email) BETWEEN 3 AND 254 AND email ~ '@'),
    notes      text        CHECK (notes IS NULL OR length(notes) <= 2000),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX customers_email_idx ON customers (lower(email));

CREATE TABLE products (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    slug       text        NOT NULL UNIQUE CHECK (slug ~ '^[a-z][a-z0-9-]{1,40}$'),
    name       text        NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE plans (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    product_id      uuid        NOT NULL REFERENCES products (id),
    name            text        NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    -- What an entitlement of this plan switches on, and its numeric limits (-1: no limit).
    features        text[]      NOT NULL DEFAULT '{}',
    limits          jsonb       NOT NULL DEFAULT '{}',
    -- How many installations may be active at once.
    max_activations integer     NOT NULL DEFAULT 1 CHECK (max_activations BETWEEN 1 AND 10000),
    -- How long a new license lasts (days); null: until it is ended.
    valid_days      integer     CHECK (valid_days IS NULL OR valid_days BETWEEN 1 AND 36500),
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (product_id, name)
);

CREATE TABLE licenses (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id  uuid        NOT NULL REFERENCES customers (id),
    plan_id      uuid        NOT NULL REFERENCES plans (id),
    -- The key the customer types in is shown once; only its SHA-256 is kept.
    key_hash     bytea       NOT NULL UNIQUE CHECK (length(key_hash) = 32),
    key_hint     text        NOT NULL CHECK (length(key_hint) <= 12),
    status       text        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'revoked')),
    -- null: no end date.
    expires_at   timestamptz,
    status_note  text        CHECK (status_note IS NULL OR length(status_note) <= 500),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX licenses_customer_idx ON licenses (customer_id);

-- One row per Built Panel installation ever seen (its identity is a key pair made at install time).
CREATE TABLE installations (
    id            uuid        PRIMARY KEY,
    public_key    bytea       NOT NULL UNIQUE CHECK (length(public_key) = 32),
    hostname      text        CHECK (hostname IS NULL OR length(hostname) <= 253),
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE activations (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    license_id      uuid        NOT NULL REFERENCES licenses (id),
    installation_id uuid        NOT NULL REFERENCES installations (id),
    activated_at    timestamptz NOT NULL DEFAULT now(),
    deactivated_at  timestamptz,
    last_heartbeat_at timestamptz,
    last_refresh_at timestamptz,
    heartbeat_info  jsonb       NOT NULL DEFAULT '{}'
);
-- An installation holds a license once at a time.
CREATE UNIQUE INDEX activations_active_idx ON activations (license_id, installation_id) WHERE deactivated_at IS NULL;
CREATE INDEX activations_license_idx ON activations (license_id) WHERE deactivated_at IS NULL;

CREATE TABLE subscriptions (
    id                 uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    license_id         uuid        NOT NULL REFERENCES licenses (id),
    state              text        NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'active', 'past_due', 'grace', 'suspended', 'cancelled', 'expired')),
    provider           text        CHECK (provider IS NULL OR length(provider) <= 40),
    provider_reference text        CHECK (provider_reference IS NULL OR length(provider_reference) <= 200),
    current_period_end timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- Every entitlement issued, so that what was signed can be shown and a jti can be looked up.
CREATE TABLE entitlements (
    jti             uuid        PRIMARY KEY,
    license_id      uuid        NOT NULL REFERENCES licenses (id),
    installation_id uuid        NOT NULL REFERENCES installations (id),
    kid             text        NOT NULL,
    issued_at       timestamptz NOT NULL,
    expires_at      timestamptz NOT NULL,
    features        text[]      NOT NULL,
    limits          jsonb       NOT NULL
);
CREATE INDEX entitlements_license_idx ON entitlements (license_id, issued_at DESC);

-- PUBLIC keys only. The private half lives in the key file or the KMS and is never written
-- here (a test looks for it).
CREATE TABLE signing_keys (
    kid        text        PRIMARY KEY CHECK (kid ~ '^[A-Za-z0-9_-]{1,64}$'),
    purpose    text        NOT NULL CHECK (purpose IN ('license', 'release')),
    public_key bytea       NOT NULL CHECK (length(public_key) = 32),
    status     text        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'retired')),
    created_at timestamptz NOT NULL DEFAULT now(),
    retired_at timestamptz
);
-- One active key per purpose.
CREATE UNIQUE INDEX signing_keys_active_idx ON signing_keys (purpose) WHERE status = 'active';

CREATE TABLE release_manifests (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    version    text        NOT NULL UNIQUE CHECK (length(version) BETWEEN 1 AND 40),
    channel    text        NOT NULL DEFAULT 'stable' CHECK (channel IN ('stable', 'beta')),
    -- The signed manifest document, as published.
    document   jsonb       NOT NULL,
    kid        text        NOT NULL REFERENCES signing_keys (kid),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE release_components (
    manifest_id uuid  NOT NULL REFERENCES release_manifests (id) ON DELETE CASCADE,
    name        text  NOT NULL CHECK (name ~ '^[a-z][a-z0-9_-]{1,40}$'),
    sha256      bytea NOT NULL CHECK (length(sha256) = 32),
    size_bytes  bigint NOT NULL CHECK (size_bytes >= 0),
    PRIMARY KEY (manifest_id, name)
);

CREATE TABLE integrity_scans (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id uuid        NOT NULL REFERENCES installations (id),
    nonce           bytea       NOT NULL,
    challenged_at   timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL,
    reported_at     timestamptz,
    result          text        CHECK (result IN ('passed', 'failed', 'partial', 'stale', 'unverifiable')),
    report          jsonb
);
CREATE INDEX integrity_scans_installation_idx ON integrity_scans (installation_id, challenged_at DESC);

CREATE TABLE integrity_events (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id uuid        NOT NULL REFERENCES installations (id),
    scan_id         uuid        REFERENCES integrity_scans (id),
    kind            text        NOT NULL CHECK (kind IN ('mismatch', 'stale', 'unverifiable', 'recovered', 'note')),
    detail          jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE revocations (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    license_id uuid        NOT NULL REFERENCES licenses (id),
    reason     text        NOT NULL CHECK (length(reason) <= 500),
    revoked_by uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE recovery_requests (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id uuid        NOT NULL REFERENCES installations (id),
    event_id        uuid        REFERENCES integrity_events (id),
    state           text        NOT NULL DEFAULT 'requested' CHECK (state IN ('requested', 'approved', 'rejected', 'completed')),
    note            text        CHECK (note IS NULL OR length(note) <= 2000),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE admin_users (
    id            uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text        NOT NULL CHECK (length(email) BETWEEN 3 AND 254 AND email ~ '@'),
    password_hash text        NOT NULL,
    role          text        NOT NULL DEFAULT 'admin' CHECK (role IN ('admin', 'viewer')),
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX admin_users_email_idx ON admin_users (lower(email));

-- Sign-ins: only the SHA-256 of the bearer token is kept.
CREATE TABLE admin_sessions (
    token_hash bytea       PRIMARY KEY CHECK (length(token_hash) = 32),
    admin_id   uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE TABLE audit_logs (
    id         bigserial   PRIMARY KEY,
    at         timestamptz NOT NULL DEFAULT now(),
    actor      text        NOT NULL,
    action     text        NOT NULL CHECK (length(action) <= 64),
    target     text,
    detail     jsonb       NOT NULL DEFAULT '{}'
);
CREATE INDEX audit_logs_at_idx ON audit_logs (at DESC);

CREATE TABLE idempotency_keys (
    key        text        NOT NULL CHECK (length(key) BETWEEN 8 AND 128),
    scope      text        NOT NULL CHECK (length(scope) <= 40),
    request_hash bytea     NOT NULL,
    response   jsonb       NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, key)
);

-- Challenges handed out before an activation or refresh (proof of possession).
CREATE TABLE activation_nonces (
    nonce           bytea       PRIMARY KEY CHECK (length(nonce) = 32),
    installation_id uuid        NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL
);
CREATE INDEX activation_nonces_expires_idx ON activation_nonces (expires_at);
