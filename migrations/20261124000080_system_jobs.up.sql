-- The panel's own background jobs (Server, Scheduled actions): each has a repeat time an administrator may
-- change, can be switched off, can be run at once, and keeps the result of its last runs.
CREATE TABLE system_jobs (
    key                   text        PRIMARY KEY CHECK (key ~ '^[a-z][a-z0-9_]{1,62}$'),
    title                 text        NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120),
    description           text        NOT NULL CHECK (char_length(description) <= 500),
    default_interval_secs integer     NOT NULL CHECK (default_interval_secs > 0),
    min_interval_secs     integer     NOT NULL CHECK (min_interval_secs > 0),
    interval_secs         integer     NOT NULL CHECK (interval_secs > 0),
    enabled               boolean     NOT NULL DEFAULT true,
    running               boolean     NOT NULL DEFAULT false,
    run_requested         boolean     NOT NULL DEFAULT false,
    last_started_at       timestamptz,
    last_finished_at      timestamptz,
    last_status           text        CHECK (last_status IN ('ok', 'error')),
    last_error            text,
    run_count             bigint      NOT NULL DEFAULT 0,
    updated_at            timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE system_job_runs (
    id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    job_key     text        NOT NULL REFERENCES system_jobs (key) ON DELETE CASCADE,
    started_at  timestamptz NOT NULL,
    finished_at timestamptz NOT NULL,
    ok          boolean     NOT NULL,
    manual      boolean     NOT NULL DEFAULT false,
    message     text        NOT NULL DEFAULT ''
);
CREATE INDEX system_job_runs_job_idx ON system_job_runs (job_key, id DESC);
