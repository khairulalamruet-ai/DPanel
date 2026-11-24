# Restoring the panel on a new server

For when the control server is lost. A system backup holds the control database (with the
audit log), `control.toml`, the files the configuration names (database URL, Redis URL, DNS
key) and the node CA. It does **not** hold the master key: **keep a copy of the master key
(`secrets.master_key_file`) somewhere other than the control server** — without it no system
backup can be opened, and the sealed secrets in the database (TOTP secrets, tenant backup
keys, destination credentials, database passwords) cannot be read.

Tested with `crates/system-backup/tests/live.rs` (dump, store, open, `pg_restore` into a new
database, rows compared) and by the drill `apps/bpctl/tests/restore_drill_live.rs`: a migrated
panel with an administrator and a CA is backed up, then restored with the real `bpctl` steps
of this runbook into a new database; every table has the same rows, the administrator's
password hash is identical, the CA is the same file and `bpctl migrate` finds nothing to apply.

## 1. Prepare the new server

1. Install the same Built Panel version (`builtpanel-control-api`, `bpctl`), PostgreSQL and Redis
   (the versions of the old server or newer), and `postgresql-client` (`pg_restore`).
2. Make an empty database and user for the panel; note the URL.

## 2. Fetch the backup

Pick the newest ready backup from the destination (the admin API lists them:
`GET /api/v1/admin/system-backups`, or look at the destination: each backup is a folder
named by its id with `system.tar.zst.enc` and `manifest.json.enc`). Download
`<id>/system.tar.zst.enc` (copy it from the folder, `aws s3 cp`, `sftp get`, ...).

## 3. Open it

Put the master key in a file (mode 0600) and open the backup into a **new** folder:

```sh
bpctl system-backup open \
  --master-key-file /root/master.key \
  --input system.tar.zst.enc \
  --output /root/restore
```

A wrong key or a damaged file stops here with an error and writes nothing useful. The folder
(mode 0700) holds `database.dump`, `config/control.toml`, `secrets/*`, `pki/node_ca.{crt,key}`.

## 4. Restore

```sh
# the database
pg_restore --no-owner --dbname "postgres://USER:PASSWORD@HOST/panel" /root/restore/database.dump

# configuration, secrets and the CA, where the old server had them
install -d -m 0755 /etc/builtpanel && install -d -m 0700 /etc/builtpanel/secrets
install -m 0640 /root/restore/config/control.toml /etc/builtpanel/control.toml
install -m 0600 /root/restore/secrets/* /etc/builtpanel/secrets/     # then fix the names the config uses
install -m 0644 /root/restore/pki/node_ca.crt /etc/builtpanel/secrets/node_ca.crt
install -m 0600 /root/restore/pki/node_ca.key /etc/builtpanel/secrets/node_ca.key
install -m 0600 /root/master.key /etc/builtpanel/secrets/master.key  # the path in secrets.master_key_file
```

Edit `control.toml` where the new server differs (database URL, Redis URL, `agent.public_url`,
`agent.stream_url`). Files under `secrets/` are named `database_url`, `redis_url`,
`powerdns_api_key`: put each at the path the configuration gives (`*_file`).

## 5. Start and check

```sh
bpctl migrate           # applies migrations newer than the backup, if the version is newer
systemctl start builtpanel-control-api
```

- Sign in as an administrator; the audit log has the history up to the backup.
- Nodes connect again with their existing certificates (the CA is the same). If the new
  server has another name or address, issue a new server certificate for the agent port
  (`bpctl ca issue-server --dns NAME --out-dir ...`); the nodes' trust is the CA, not the
  server certificate.
- Everything done after the backup is not in it: operations queued since then are lost, and
  the nodes may hold newer state than the database. The node reports (`report_*` operations)
  bring most of it back within minutes; check domains, databases and certificates of the
  accounts that changed in the last hours.
- Tenant backups (`GET /api/v1/client/backups`) come back with the database; their keys are
  sealed with the master key, so they restore as before.

## What a scheduled system backup does

`PUT /api/v1/admin/system-backup-schedule` sets a cron (UTC, at most hourly) and how many
days and weeks are kept; the newest backup is always kept. Failed runs show as `failed` with
the reason in `GET /api/v1/admin/system-backups`.
