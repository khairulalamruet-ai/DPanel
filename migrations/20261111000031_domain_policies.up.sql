-- T06.05: what a domain's web server does besides serving files: a rate limit and
-- extra response headers. They are kept apart from `domains` (most domains have
-- none) and change the tenant's routes version, so the configuration is sent again.
CREATE TABLE domain_policies (
    domain_id             uuid PRIMARY KEY REFERENCES domains (id) ON DELETE CASCADE,
    rate_limit_per_second smallint CHECK (rate_limit_per_second BETWEEN 1 AND 1000),
    headers               jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(headers) = 'array' AND jsonb_array_length(headers) <= 10),
    updated_at            timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER domain_policies_updated_at
    BEFORE UPDATE ON domain_policies
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE FUNCTION domain_policies_routes_changed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE tenants SET routes_version = routes_version + 1
     WHERE id = (SELECT tenant_id FROM domains WHERE id = COALESCE(NEW.domain_id, OLD.domain_id));
    RETURN NULL;
END;
$$;

CREATE TRIGGER domain_policies_routes_changed
    AFTER INSERT OR UPDATE OR DELETE ON domain_policies
    FOR EACH ROW EXECUTE FUNCTION domain_policies_routes_changed();

-- The default template gets the two places the policy goes (only when nobody
-- has edited it; a test keeps this equal to builtpanel_domain::vhost).
UPDATE vhost_templates SET body = $new$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
{{policy_http}}
server {
    listen 8080;
    server_name {{server_names}};
    root {{docroot}};
    index index.php index.html index.htm;
    {{policy}}

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \.php$ {
        try_files $uri =404;
        include fastcgi_params;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        fastcgi_pass {{php_upstream}};
    }

    location ~ /\.(?!well-known) {
        deny all;
    }
}
$new$
 WHERE server = 'nginx' AND name = 'Default' AND body = $old$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
server {
    listen 8080;
    server_name {{server_names}};
    root {{docroot}};
    index index.php index.html index.htm;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \.php$ {
        try_files $uri =404;
        include fastcgi_params;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        fastcgi_pass {{php_upstream}};
    }

    location ~ /\.(?!well-known) {
        deny all;
    }
}
$old$;
