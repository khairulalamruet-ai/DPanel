# Caddy with the Coraza WAF module and the OWASP Core Rule Set (T11.01).
#
# Built by .github/workflows/rust-images.yml (and by deploy/dev/waf-live.sh for a test);
# the base images are pinned by digest. The result runs as the unprivileged user `caddy`,
# keeps its data in /data and /config, and listens on whatever its configuration says.
#
# coraza-caddy v2.x: https://github.com/corazawaf/coraza-caddy  (Apache-2.0)
# OWASP Core Rule Set: https://github.com/coreruleset/coreruleset  (Apache-2.0)
ARG CADDY_VERSION=2.10.2
ARG CORAZA_CADDY_VERSION=v2.1.0
ARG CRS_VERSION=v4.17.1

FROM caddy:${CADDY_VERSION}-builder AS builder
ARG CORAZA_CADDY_VERSION
RUN xcaddy build \
      --with github.com/corazawaf/coraza-caddy/v2@${CORAZA_CADDY_VERSION}

FROM alpine:3.22 AS rules
ARG CRS_VERSION
RUN apk add --no-cache curl tar \
 && mkdir -p /crs \
 && curl -fsSL "https://github.com/coreruleset/coreruleset/archive/refs/tags/${CRS_VERSION}.tar.gz" \
      | tar -xz -C /crs --strip-components=1

FROM caddy:${CADDY_VERSION}-alpine
COPY --from=builder /usr/bin/caddy /usr/bin/caddy
COPY coraza.conf /etc/caddy/coraza.conf
COPY --from=rules /crs/rules /etc/caddy/crs/rules
COPY --from=rules /crs/crs-setup.conf.example /etc/caddy/crs/crs-setup.conf
RUN caddy list-modules | grep -q http.handlers.waf
# The firewall's audit log (coraza.conf); mount a directory of the host here for the node agent.
RUN mkdir -p /var/log/caddy
VOLUME /var/log/caddy
