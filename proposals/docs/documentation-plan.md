# Documentation plan (T19.10)

Status: draft. Everything is written twice: English and Bangla (the Bangla text is a translation
of the same structure, page for page). A documentation site is built from Markdown (one folder
per language) and published with the release.

## Guides
| Guide | Reader | Contents |
|---|---|---|
| Install | whoever sets up a server | requirements (supported systems, ports, DNS), the one-line install, what the installer does, the first sign-in, adding a node, switching on HTTPS, upgrading, uninstalling |
| Administrator | the panel's operator | plans, accounts, domains and DNS, nodes, backups and destinations, security policy, the licence, notifications, updates, resellers, migration |
| Client | a hosting customer | first sign-in, domains, files and FTP, databases, email, backups, cron, security, applications |
| Reseller | a reseller | packages, plans, creating clients, branding |
| API | integrators | generated from `openapi.json`; authentication and CSRF, errors, pagination, examples |
| Module authoring | developers | `docs/guides/creating-a-module.md` (T03) |

## Runbooks (operator, short, tested)
1. **Incident:** how to tell what is wrong (`/api/v1/public/health`, `bpctl status`, node
   status, the logs), a decision list (a node offline, the database full, a licence problem), who
   to call.
2. **Key rotation:** the release key (`bpctl release` + the `release-public-keys.txt` ring), the
   node CA, the master key for stored secrets, the licence authority's signing key. For each:
   when, the exact commands, what to check afterwards, how to roll back.
3. **Restore:** the control plane's database from its backup, one tenant from a backup,
   a node rebuilt from nothing, what is lost in each case.
4. **Update and rollback:** staging first, `builtpanel-deploy`, what a failed update leaves behind.
5. **Licence:** an expired or revoked licence, a hardware change, the integrity scan's
   `recovering` state and how an administrator approves or rejects recovery.

## Style
Short steps, one command per line, the expected output next to it. Every command in the
documents is run by `deploy/dev/*-live.sh` or by hand on a fresh server before release; a command
that was not run is not in the guide.

## Publishing
Built in CI on every tag, published next to the release artifacts, versioned with the panel.
