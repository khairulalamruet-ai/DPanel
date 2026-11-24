-- T07.02: the databases, database users and grants of an account (see
-- builtpanel_domain::databases for the naming rules). Passwords are never kept: the
-- node gets one when a user is made or changed and forgets it.
CREATE TABLE databases (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    tenant_id       uuid        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name            text        NOT NULL UNIQUE
        CONSTRAINT databases_name_format CHECK (name ~ '^[a-z0-9_]{1,64}$'),
    status          text        NOT NULL DEFAULT 'pending'
        CONSTRAINT databases_status CHECK (status IN ('pending', 'ready', 'failed', 'deleting')),
    size_bytes      bigint      NOT NULL DEFAULT 0 CHECK (size_bytes >= 0),
    size_checked_at timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, tenant_id)
);
CREATE INDEX databases_tenant_idx ON databases (tenant_id);

CREATE TABLE db_users (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    tenant_id  uuid        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name       text        NOT NULL UNIQUE
        CONSTRAINT db_users_name_format CHECK (name ~ '^[a-z0-9_]{1,64}$'),
    status     text        NOT NULL DEFAULT 'pending'
        CONSTRAINT db_users_status CHECK (status IN ('pending', 'ready', 'failed', 'deleting')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, tenant_id)
);
CREATE INDEX db_users_tenant_idx ON db_users (tenant_id);

-- Who may do what in which database. The account is in both keys, so a user of one
-- account can never be given a database of another.
CREATE TABLE db_grants (
    tenant_id   uuid NOT NULL,
    database_id uuid NOT NULL,
    user_id     uuid NOT NULL,
    access      text NOT NULL
        CONSTRAINT db_grants_access CHECK (access IN ('read_only', 'read_write', 'all')),
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (database_id, user_id),
    FOREIGN KEY (database_id, tenant_id) REFERENCES databases (id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, tenant_id) REFERENCES db_users (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX db_grants_user_idx ON db_grants (user_id);
