# The web server image of a tenant stack (nginx, unprivileged): the official image with the
# operating system's packages brought up to date, so what the distribution has fixed since the
# official image was built is fixed here too. Nothing else is changed. Built by
# .github/workflows/rust-images.yml (and deploy/dev/build-images.sh) with BASE set to the
# pinned official image of this role in deploy/stack/images.toml.
ARG BASE
FROM ${BASE}
USER root
RUN apk upgrade --no-cache
# The official image runs as this user (nginx): so does this one.
USER 101
LABEL org.opencontainers.image.title="Built Panel web (nginx)" \
      org.opencontainers.image.description="The official unprivileged nginx with its OS packages upgraded"
