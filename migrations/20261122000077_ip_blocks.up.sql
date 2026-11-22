-- Addresses a hosting account turns away from one of its domains. The web server answers them 403
-- before the firewall or the site sees the request. An entry is one address or a network in CIDR
-- form, kept in its canonical text (so the same network cannot be listed twice).
CREATE TABLE domain_ip_blocks (
    id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    domain_id  uuid        NOT NULL REFERENCES domains (id) ON DELETE CASCADE,
    cidr       text        NOT NULL CHECK (length(cidr) BETWEEN 2 AND 43),
    note       text        CHECK (note IS NULL OR (length(note) <= 200 AND note !~ '[[:cntrl:]]')),
    created_by uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (domain_id, cidr)
);
CREATE INDEX domain_ip_blocks_tenant_idx ON domain_ip_blocks (tenant_id);
