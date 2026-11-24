UPDATE vhost_templates SET body = $old$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
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
$old$
 WHERE server = 'nginx' AND name = 'Default' AND body = $new$# Managed by Built Panel for {{domain}}. Changes made here are overwritten.
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
$new$;
DROP TABLE IF EXISTS domain_custom_blocks;
DROP FUNCTION IF EXISTS domain_custom_blocks_routes_changed();
