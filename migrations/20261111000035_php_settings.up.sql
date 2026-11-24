-- T06.10: the php.ini options of an account (see builtpanel_domain::php_ini). One
-- row per account that changed anything; a change makes the account's files and
-- settings out of date so the node is told.
CREATE TABLE php_settings (
    tenant_id  uuid PRIMARY KEY REFERENCES tenants (id) ON DELETE CASCADE,
    options    jsonb       NOT NULL DEFAULT '{}'::jsonb
        CONSTRAINT php_settings_object CHECK (jsonb_typeof(options) = 'object'),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER php_settings_updated_at
    BEFORE UPDATE ON php_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE FUNCTION php_settings_routes_changed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE tenants SET routes_version = routes_version + 1
     WHERE id = COALESCE(NEW.tenant_id, OLD.tenant_id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER php_settings_routes_changed
    AFTER INSERT OR UPDATE OR DELETE ON php_settings
    FOR EACH ROW EXECUTE FUNCTION php_settings_routes_changed();

-- A failed set_php_ini puts the account back on the list, like the others.
CREATE OR REPLACE FUNCTION operations_apply_to_tenant() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS NULL THEN
        RETURN NEW;
    END IF;
    IF NEW.kind = 'set_up_tenant' THEN
        UPDATE tenants
           SET status = CASE WHEN NEW.status = 'succeeded' THEN 'active' ELSE 'failed' END
         WHERE id = NEW.tenant_id AND status = 'provisioning';
    ELSIF NEW.kind = 'suspend_tenant' AND NEW.status = 'succeeded' THEN
        UPDATE tenants SET status = 'suspended' WHERE id = NEW.tenant_id AND status = 'active';
    ELSIF NEW.kind = 'resume_tenant' AND NEW.status = 'succeeded' THEN
        UPDATE tenants SET status = 'active' WHERE id = NEW.tenant_id AND status = 'suspended';
    ELSIF NEW.kind = 'delete_tenant' AND NEW.status = 'succeeded'
          AND COALESCE((NEW.payload ->> 'purge_data')::boolean, false) THEN
        DELETE FROM tenants WHERE id = NEW.tenant_id AND status = 'deleting';
    ELSIF NEW.kind = 'set_php_versions' AND NEW.status = 'succeeded' THEN
        -- `php:<tenant>:<state>:<slot>`
        UPDATE tenants
           SET php_applied_version = GREATEST(php_applied_version,
                                              split_part(NEW.idempotency_key, ':', 3)::bigint),
               routes_synced_version = 0
         WHERE id = NEW.tenant_id AND NEW.idempotency_key ~ '^php:[0-9a-f]+:[0-9]+:[0-9]+$';
    ELSIF NEW.kind IN ('set_site_routes', 'set_custom_certificates', 'attach_domain',
                       'keep_domains', 'set_php_versions', 'set_php_ini')
          AND NEW.status = 'failed' THEN
        UPDATE tenants SET routes_synced_version = 0 WHERE id = NEW.tenant_id;
    END IF;
    RETURN NEW;
END;
$$;
