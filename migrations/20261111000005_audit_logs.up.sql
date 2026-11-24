-- Append-only audit log (T01.12).
--
-- Rows are only ever inserted. UPDATE, DELETE and TRUNCATE raise an error for
-- every role, including the table owner (the triggers below); only someone
-- who can ALTER the table (the migration role, never the application) could
-- disable them. There are deliberately no foreign keys: an audit row must
-- outlive the account, tenant or node it mentions, and a cascading change
-- would itself be an UPDATE of the log.
CREATE TABLE audit_logs (
    id              uuid        PRIMARY KEY DEFAULT uuidv7(),
    at              timestamptz NOT NULL DEFAULT now(),
    actor_id        uuid,
    actor_ip        inet,
    action          text        NOT NULL
        CONSTRAINT audit_logs_action_format CHECK (action ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$'),
    target_type     text
        CONSTRAINT audit_logs_target_type_len CHECK (length(target_type) BETWEEN 1 AND 64),
    target_id       text
        CONSTRAINT audit_logs_target_id_len CHECK (length(target_id) BETWEEN 1 AND 128),
    tenant_id       uuid,
    node_id         uuid,
    outcome         text        NOT NULL
        CONSTRAINT audit_logs_outcome CHECK (outcome IN ('success', 'failure', 'denied')),
    correlation_id  text
        CONSTRAINT audit_logs_correlation_len CHECK (length(correlation_id) <= 128),
    metadata        jsonb       NOT NULL DEFAULT '{}'
        CONSTRAINT audit_logs_metadata_object CHECK (jsonb_typeof(metadata) = 'object')
        CONSTRAINT audit_logs_metadata_size CHECK (octet_length(metadata::text) <= 8192)
);

-- Newest first, then by who, what and which object.
CREATE INDEX audit_logs_at_idx ON audit_logs (at DESC, id DESC);
CREATE INDEX audit_logs_actor_idx ON audit_logs (actor_id, at DESC, id DESC) WHERE actor_id IS NOT NULL;
CREATE INDEX audit_logs_action_idx ON audit_logs (action text_pattern_ops, at DESC, id DESC);
CREATE INDEX audit_logs_target_idx ON audit_logs (target_type, target_id, at DESC) WHERE target_id IS NOT NULL;

CREATE FUNCTION audit_logs_reject_change() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_logs is append-only: % is not allowed', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

CREATE TRIGGER audit_logs_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_logs
    FOR EACH ROW EXECUTE FUNCTION audit_logs_reject_change();

CREATE TRIGGER audit_logs_no_truncate
    BEFORE TRUNCATE ON audit_logs
    FOR EACH STATEMENT EXECUTE FUNCTION audit_logs_reject_change();

-- Production uses separate roles (docs/01-architecture/06-database-schema.md):
-- the migration role owns the table and the application role may only read and
-- insert. Applied when the role already exists; otherwise grant it by hand:
--   REVOKE ALL ON audit_logs FROM builtpanel_app;
--   GRANT SELECT, INSERT ON audit_logs TO builtpanel_app;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'builtpanel_app') THEN
        REVOKE ALL ON audit_logs FROM builtpanel_app;
        GRANT SELECT, INSERT ON audit_logs TO builtpanel_app;
    END IF;
END
$$;
