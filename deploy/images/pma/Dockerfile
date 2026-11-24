# The node's database browser (phpMyAdmin, Apache): the official image with the operating
# system's packages brought up to date and the libraries phpMyAdmin carries (Twig, Symfony)
# moved to their newest releases the application allows, which is where the official image's
# fixable findings are. The entrypoint and the user are the official image's. Built with BASE
# set to the pinned official image (see deploy/images/web/Dockerfile).
ARG BASE
FROM ${BASE} AS base
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get -y --no-install-recommends upgrade \
    && rm -rf /var/lib/apt/lists/*

# Composer works on a copy of the application's own libraries, so the final image gets
# nothing but the updated vendor folder.
FROM composer:2 AS libraries
COPY --from=base /var/www/html /app
WORKDIR /app
# Only what phpMyAdmin's own constraints allow (no major jump): a library that cannot move
# stays where it is. The PHP extensions are checked where the image runs, not in this stage.
RUN composer update --no-dev --no-interaction --no-scripts --optimize-autoloader \
        --ignore-platform-reqs --with-all-dependencies \
        twig/twig symfony/cache symfony/expression-language symfony/polyfill-mbstring symfony/polyfill-ctype

FROM base
COPY --from=libraries /app/vendor /var/www/html/vendor
LABEL org.opencontainers.image.title="Built Panel database browser (phpMyAdmin)" \
      org.opencontainers.image.description="The official phpMyAdmin with its OS packages upgraded and its libraries updated"
