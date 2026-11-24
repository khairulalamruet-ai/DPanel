-- Moving a whole server to another one: the panel logs in to the new server over SSH, installs a node
-- there and joins it to this panel (the accounts are then moved with the node transfers). The SSH
-- password is never stored; the log holds the installer's output with every secret removed.
CREATE TABLE server_migrations (
    id          uuid        PRIMARY KEY,
    host        text        NOT NULL CHECK (char_length(host) BETWEEN 1 AND 253),
    port        integer     NOT NULL CHECK (port BETWEEN 1 AND 65535),
    ssh_user    text        NOT NULL CHECK (char_length(ssh_user) BETWEEN 1 AND 32),
    node_name   text        NOT NULL,
    status      text        NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed')),
    phase       text        NOT NULL DEFAULT 'connecting',
    node_id     uuid        REFERENCES nodes (id) ON DELETE SET NULL,
    log         text        NOT NULL DEFAULT '',
    created_by  uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX server_migrations_created_idx ON server_migrations (created_at DESC);
