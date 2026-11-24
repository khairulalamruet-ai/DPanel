# The node's SFTP and FTPS server (SFTPGo): the official image with the operating system's
# packages brought up to date. The program inside is the official one, untouched. Built with
# BASE set to the pinned official image (see deploy/images/web/Dockerfile).
ARG BASE
FROM ${BASE}
USER root
RUN apk upgrade --no-cache
# The official image runs as this user: the agent starts the container as root anyway
# (SFTPGo changes to each account's id), but the default stays what it was.
USER 1000:1000
LABEL org.opencontainers.image.title="Built Panel SFTP (SFTPGo)" \
      org.opencontainers.image.description="The official SFTPGo with its OS packages upgraded"
