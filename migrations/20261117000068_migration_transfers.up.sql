-- T15.04, T15.06: an import of another panel's backup, with what it did step by step.
CREATE TABLE migration_transfers (
    id               uuid        PRIMARY KEY DEFAULT uuidv7(),
    source_type      text        NOT NULL
        CHECK (source_type IN ('cpanel', 'cyberpanel', 'openpanel', 'builtpanel_archive')),
    -- Where the backup came from, when it was fetched from a server (T15.05).
    source_host      text        CHECK (length(source_host) <= 255),
    -- The backup's file name inside the panel's import folder.
    backup_file      text        NOT NULL
        CHECK (backup_file ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,199}$' AND backup_file !~ '\.\.'),
    tenant_username  text        NOT NULL CHECK (length(tenant_username) BETWEEN 1 AND 64),
    target_domain    text        NOT NULL CHECK (length(target_domain) BETWEEN 1 AND 253),
    dry_run          boolean     NOT NULL DEFAULT true,
    on_conflict      text        NOT NULL DEFAULT 'fail' CHECK (on_conflict IN ('fail', 'rename', 'skip')),
    status           text        NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    progress_percent integer     NOT NULL DEFAULT 0 CHECK (progress_percent BETWEEN 0 AND 100),
    current_step     text        NOT NULL DEFAULT '' CHECK (length(current_step) <= 200),
    size_bytes       bigint      CHECK (size_bytes >= 0),
    -- [{id, name, status, started_at?, completed_at?, details?}]
    steps            jsonb       NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(steps) = 'array'),
    -- The last lines of the log (the application keeps at most 2000).
    logs             jsonb       NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(logs) = 'array'),
    -- The dry run's answer: what the import does and what stands in its way.
    plan             jsonb,
    error_message    text        CHECK (length(error_message) <= 1000),
    created_by       uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX migration_transfers_created_idx ON migration_transfers (created_at DESC);
