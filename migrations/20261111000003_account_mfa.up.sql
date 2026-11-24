-- Second factors (T01.08).
--
-- account_mfa: one row per account and factor kind. `secret_enc` is the TOTP
-- secret encrypted with the master key (XChaCha20-Poly1305, see
-- builtpanel_auth_core::secretbox); the plaintext never reaches the database.
-- A row with confirmed_at IS NULL is an enrolment in progress and does not
-- protect logins yet. `last_used_step` is the newest accepted TOTP time step;
-- codes for that step or older are refused (replay protection).
CREATE TABLE account_mfa (
    id              uuid        PRIMARY KEY DEFAULT uuidv7(),
    account_id      uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind            text        NOT NULL
        CONSTRAINT account_mfa_kind CHECK (kind IN ('totp')),
    secret_enc      bytea       NOT NULL
        CONSTRAINT account_mfa_secret_enc_len CHECK (octet_length(secret_enc) BETWEEN 42 AND 512),
    confirmed_at    timestamptz,
    last_used_step  bigint
        CONSTRAINT account_mfa_step_nonneg CHECK (last_used_step >= 0),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT account_mfa_one_per_kind UNIQUE (account_id, kind)
);

CREATE TRIGGER account_mfa_set_updated_at
    BEFORE UPDATE ON account_mfa
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Single-use recovery codes, stored as SHA-256 of the normalised code
-- (the codes are random, ~49.5 bits, so a fast hash is sufficient).
CREATE TABLE mfa_backup_codes (
    id           uuid        PRIMARY KEY DEFAULT uuidv7(),
    account_id   uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    code_sha256  bytea       NOT NULL
        CONSTRAINT mfa_backup_codes_hash_len CHECK (octet_length(code_sha256) = 32),
    used_at      timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT mfa_backup_codes_unique UNIQUE (account_id, code_sha256)
);
