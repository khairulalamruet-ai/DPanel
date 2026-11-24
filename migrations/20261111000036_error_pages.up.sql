-- T06.13: pages an administrator wrote for the cases a site cannot answer
-- (see builtpanel_domain::error_pages). A kind without a row uses the installation's
-- own page.
CREATE TABLE error_pages (
    kind       text PRIMARY KEY
        CHECK (kind IN ('suspended_domain', 'suspended_account', 'not_found', 'bad_gateway')),
    body       text        NOT NULL CHECK (length(body) BETWEEN 1 AND 16384),
    updated_by uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER error_pages_updated_at
    BEFORE UPDATE ON error_pages
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
