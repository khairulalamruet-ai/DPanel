# The PHP-FPM image tenants run (T06.01): the official PHP image plus the
# extensions most sites need. Built and published by .github/workflows/rust-images.yml.
#
#   docker build --build-arg PHP_VERSION=8.4 -t builtpanel-php:8.4 deploy/images/php
#
# Nothing here runs as root at run time: the panel starts the container as the
# tenant's own user id, and the image needs no capability.
ARG PHP_VERSION=8.4
FROM php:${PHP_VERSION}-fpm-alpine

# The operating system's packages, up to date (what the distribution fixed after the official
# image was built).
RUN apk upgrade --no-cache

# https://github.com/mlocati/docker-php-extension-installer
COPY --from=ghcr.io/mlocati/php-extension-installer:2.13.2 /usr/bin/install-php-extensions /usr/local/bin/

# Common extensions: databases, images, text, archives, caching.
RUN install-php-extensions \
        bcmath exif gd intl mysqli opcache pdo_mysql pdo_pgsql pgsql sockets zip \
    && rm -f /usr/local/bin/install-php-extensions \
    && rm -rf /tmp/* /var/cache/apk/*

LABEL org.opencontainers.image.title="Built Panel PHP-FPM" \
      org.opencontainers.image.description="PHP-FPM with the extensions hosting sites usually need" \
      org.opencontainers.image.source="https://github.com/builtpanel"
EXPOSE 9000
