-- T06.06: the directives a person adds to their own site (`edit_vhost`). One row
-- per domain that has any; changing it makes the tenant's routes out of date so
-- the site's file is sent again.
CREATE TABLE domain_custom_blocks (
    domain_id  uuid PRIMARY KEY REFERENCES domains (id) ON DELETE CASCADE,
    body       text NOT NULL CHECK (length(body) BETWEEN 1 AND 8192),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER domain_custom_blocks_updated_at
    BEFORE UPDATE ON domain_custom_blocks
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE FUNCTION domain_custom_blocks_routes_changed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE tenants SET routes_version = routes_version + 1
     WHERE id = (SELECT tenant_id FROM domains WHERE id = COALESCE(NEW.domain_id, OLD.domain_id));
    RETURN NULL;
END;
$$;

CREATE TRIGGER domain_custom_blocks_routes_changed
    AFTER INSERT OR UPDATE OR DELETE ON domain_custom_blocks
    FOR EACH ROW EXECUTE FUNCTION domain_custom_blocks_routes_changed();

-- The default template gets the place for it (only when nobody has edited it).
UPDATE vhost_templates SET body = $new$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
{{policy_http}}
server {
    listen 8080;
    server_name {{server_names}};
    root {{docroot}};
    index index.php index.html index.htm;
    {{policy}}
    {{custom}}

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
$old$;
