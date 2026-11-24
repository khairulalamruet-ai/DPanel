-- Built Panel baseline (T00.07).
-- Requires PostgreSQL 18+ for the built-in uuidv7() used as the default
-- primary key generator by every later table.
DO $$
BEGIN
    IF current_setting('server_version_num')::int < 180000 THEN
        RAISE EXCEPTION 'Built Panel requires PostgreSQL 18 or newer (found %)', current_setting('server_version');
    END IF;
END
$$;

-- Single-row table describing this installation's schema.
CREATE TABLE schema_info (
    id          smallint    PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    product     text        NOT NULL DEFAULT 'builtpanel',
    created_at  timestamptz NOT NULL DEFAULT now()
);

INSERT INTO schema_info DEFAULT VALUES;
