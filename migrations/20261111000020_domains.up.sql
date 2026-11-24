-- T05.01: the domains a tenant hosts, and their redirects.
--
-- A name is stored lower-case in its ASCII form (punycode for international
-- names) and is unique across the whole panel: one site answers for one name.
-- The checks below repeat what `builtpanel_domain::Fqdn` / `Docroot` decide, so a
-- row that skipped the application cannot get in either.
--
-- Kinds (crates/domain, `DomainKind`):
--   primary  the tenant's main domain, exactly one per tenant
--   addon    another registrable domain with its own site
--   sub      a subdomain of one of the tenant's domains, own docroot
--   alias    another name for the site of an existing domain
-- `sub` and `alias` name the domain they belong to (`parent_domain_id`); the
-- others do not. Deleting a domain deletes what hangs off it.
CREATE TABLE domains (
    id               uuid        PRIMARY KEY DEFAULT uuidv7(),
    tenant_id        uuid        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    fqdn             text        NOT NULL
        CONSTRAINT domains_fqdn_format CHECK (
            length(fqdn) <= 253
            AND fqdn ~ '^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'
            AND fqdn !~ '\.[0-9]+$'
        ),
    kind             text        NOT NULL
        CONSTRAINT domains_kind CHECK (kind IN ('primary', 'addon', 'sub', 'alias')),
    parent_domain_id uuid        REFERENCES domains (id) ON DELETE CASCADE,
    -- Relative to the tenant's web directory; empty is that directory itself.
    docroot          text        NOT NULL DEFAULT ''
        CONSTRAINT domains_docroot_format CHECK (
            docroot = ''
            OR (docroot ~ '^[A-Za-z0-9._-]{1,64}(/[A-Za-z0-9._-]{1,64}){0,5}$'
                AND docroot !~ '(^|/)\.{1,2}(/|$)')
        ),
    php_version      text
        CONSTRAINT domains_php_version_len CHECK (length(php_version) BETWEEN 1 AND 16),
    status           text        NOT NULL DEFAULT 'active'
        CONSTRAINT domains_status CHECK (status IN ('active', 'suspended')),
    suspended_at     timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT domains_parent_by_kind CHECK ((kind IN ('sub', 'alias')) = (parent_domain_id IS NOT NULL)),
    CONSTRAINT domains_suspended_has_time CHECK ((status = 'suspended') = (suspended_at IS NOT NULL)),
    CONSTRAINT domains_not_its_own_parent CHECK (parent_domain_id IS DISTINCT FROM id)
);

CREATE UNIQUE INDEX domains_fqdn_key ON domains (fqdn);
CREATE UNIQUE INDEX domains_one_primary_per_tenant ON domains (tenant_id) WHERE kind = 'primary';
CREATE INDEX domains_tenant_idx ON domains (tenant_id, kind);
CREATE INDEX domains_parent_idx ON domains (parent_domain_id) WHERE parent_domain_id IS NOT NULL;

CREATE TRIGGER domains_set_updated_at
    BEFORE UPDATE ON domains
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- What hangs off another domain stays in the same tenant, and a subdomain's
-- name lies below its parent's. An alias cannot be the parent of anything.
CREATE FUNCTION domains_parent_check() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    parent domains%ROWTYPE;
BEGIN
    IF NEW.parent_domain_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT * INTO parent FROM domains WHERE id = NEW.parent_domain_id;
    IF parent.tenant_id IS DISTINCT FROM NEW.tenant_id THEN
        RAISE EXCEPTION 'a domain belongs to a domain of its own tenant' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF parent.kind = 'alias' THEN
        RAISE EXCEPTION 'an alias cannot have a subdomain or alias of its own' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.kind = 'sub' AND right(NEW.fqdn, length(parent.fqdn) + 1) <> '.' || parent.fqdn THEN
        RAISE EXCEPTION 'a subdomain must lie below its parent' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER domains_parent_check
    BEFORE INSERT OR UPDATE OF tenant_id, kind, fqdn, parent_domain_id ON domains
    FOR EACH ROW EXECUTE FUNCTION domains_parent_check();

-- Redirects of a domain: a path (or the whole site, "/") sent elsewhere.
CREATE TABLE redirects (
    id           uuid        PRIMARY KEY DEFAULT uuidv7(),
    domain_id    uuid        NOT NULL REFERENCES domains (id) ON DELETE CASCADE,
    -- Absolute path, no query or fragment, no spaces or control characters.
    source_path  text        NOT NULL
        CONSTRAINT redirects_source_format CHECK (
            length(source_path) <= 512 AND source_path ~ '^/[^[:space:][:cntrl:]?#]*$'
        ),
    -- An http(s) address or a path on the same site.
    target       text        NOT NULL
        CONSTRAINT redirects_target_format CHECK (
            length(target) <= 2048 AND target ~ '^(https?://[^[:space:][:cntrl:]]+|/[^[:space:][:cntrl:]]*)$'
        ),
    code         smallint    NOT NULL DEFAULT 301
        CONSTRAINT redirects_code CHECK (code IN (301, 302, 307, 308)),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX redirects_domain_source_key ON redirects (domain_id, source_path);

CREATE TRIGGER redirects_set_updated_at
    BEFORE UPDATE ON redirects
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
