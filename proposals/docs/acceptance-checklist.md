# Production acceptance checklist (T19.12)

Status: draft. The roadmap asks for the **12 items of part 17 of the licence PDF**
(`doc-guide/complete_license_anti_nulled_architecture.pdf`). The PDF could not be read by the
tooling used for this draft, so the list below is built from what the system does, one line per
property. **Reconcile it with the PDF's twelve items before signing**: keep the PDF's wording, use
the "How to verify" column to fill in the evidence.

| # | Property | How to verify | Evidence | Signed |
|---|---|---|---|---|
| 1 | The licence is signed (Ed25519), and the panel refuses an unsigned or altered one | `crates/license` tests; activate with an altered file on a test install | | |
| 2 | A licence is bound to its domain, hostnames and addresses; another host is refused | activate from a second host (`apps/license-authority` binding tests, `deploy/dev/deploy-live.sh`) | | |
| 3 | Activation proves possession of the installation key (nonce signed) | LA tests for the activation challenge | | |
| 4 | Revoking a licence takes effect at the next check; the panel degrades as designed | revoke in the authority; watch the panel's `license` state | | |
| 5 | Offline grace period works and ends | stop the authority, advance the clock | | |
| 6 | The release is signed; the server verifies the manifest before it installs anything | `bpctl release verify`; try a tampered archive with `builtpanel-deploy` | | |
| 7 | Critical binaries are checked daily against the signed registry; a changed binary is reported and recovery needs an administrator | integrity scan live test | | |
| 8 | The control plane never runs as root; the agent runs typed operations only; only the agent reaches Docker | `ps`, unit files, `deploy/dev/isolation-live.sh` | | |
| 9 | No secret in code, logs, commits or plaintext columns | secret scan in CI, `deploy/dev/audit.sh`, a read of the database dump | | |
| 10 | Sessions are `HttpOnly` cookies with CSRF; nothing in browser storage | browser check on both applications | | |
| 11 | Backups restore (files, databases, DNS), including onto an empty node | `deploy/dev/backup-live.sh`, one manual full restore | | |
| 12 | A clean install and an update on the supported systems work end to end | `deploy/dev/install-e2e.py` on each supported OS; update through staging | | |

Sign-off: name, date, release id (manifest signature), where the evidence is stored.
