-- T06.03: web server configuration templates. An administrator keeps them; the
-- control plane renders one for each domain (see builtpanel_domain::vhost). The
-- template marked as the default is the one new sites use.
CREATE TABLE vhost_templates (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    server     text        NOT NULL DEFAULT 'nginx' CHECK (server IN ('nginx')),
    name       text        NOT NULL CHECK (name ~ '^[A-Za-z0-9][A-Za-z0-9 ._-]{0,63}$'),
    body       text        NOT NULL CHECK (length(body) BETWEEN 1 AND 32768),
    is_default boolean     NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (server, name)
);
CREATE UNIQUE INDEX vhost_templates_one_default ON vhost_templates (server) WHERE is_default;

CREATE TRIGGER vhost_templates_updated_at
    BEFORE UPDATE ON vhost_templates
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The template new installations start with. A test keeps it equal to
-- builtpanel_domain::vhost::DEFAULT_NGINX_TEMPLATE.
INSERT INTO vhost_templates (name, body, is_default)
VALUES ('Default', $tpl$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
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
$tpl$, true);
