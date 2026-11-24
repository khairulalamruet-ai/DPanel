# The node's shared database server (MariaDB): the official image with the operating system's
# packages brought up to date and its one outside program, gosu, built again with a current Go
# (the copy inside the official image carries an old Go standard library, which is where
# every fixable finding of the official image is). The entrypoint, the user and MariaDB itself
# are the official image's. Built with BASE set to the pinned official image (see
# deploy/images/web/Dockerfile).
ARG BASE
FROM golang:1-alpine AS gosu
# gosu drops from root to the database user when the container starts.
ARG GOSU_VERSION=1.19
RUN CGO_ENABLED=0 go install -trimpath -ldflags="-s -w" "github.com/tianon/gosu@${GOSU_VERSION}"

FROM ${BASE}
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get -y --no-install-recommends upgrade \
    && rm -rf /var/lib/apt/lists/*
COPY --from=gosu /go/bin/gosu /usr/local/bin/gosu
RUN gosu --version && gosu nobody true
LABEL org.opencontainers.image.title="Built Panel database (MariaDB)" \
      org.opencontainers.image.description="The official MariaDB with its OS packages upgraded and gosu rebuilt"
