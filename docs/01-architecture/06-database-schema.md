# ০৬ — Database Schema (PostgreSQL)

এখানে শুধু টেবিল, মূল field আর সম্পর্ক আছে। আসল SQL migration লেখার সময় তৈরি হবে।
**নিয়ম (PDF):** UUID (v7) primary key, foreign key, unique, index, transaction। Audit append-only। Tenant-scoped query। Least-privilege DB role (`builtpanel_app`, `builtpanel_migrate`, `builtpanel_readonly`)।

## Core
| টেবিল | মূল field | মন্তব্য |
|---|---|---|
| `accounts` | id, kind(`admin`/`reseller`/`client`), username, email, password_hash(argon2id), status, locale, created_at | সব লগইন করা মানুষ |
| `account_mfa` | account_id, kind(`totp`/`passkey`), secret_enc, created_at | secret এনক্রিপ্টেড |
| `sessions` | id, account_id, ip, user_agent, created_at, last_seen_at, revoked_at | Redis-এ hot, PG-তে ইতিহাস |
| `login_events` | id, account_id, ip, country, success, reason, at | login_history |
| `roles` / `role_permissions` / `account_roles` | | RBAC |
| `resellers` | account_id, parent_id, limits jsonb | পরে |
| `tenants` | id, owner_account_id, node_id, plan_id, username(system), uid, status(`active`/`suspended`/`deleting`), created_at | এক tenant = এক container stack |
| `plans` | id, name, feature_set_id, cpu, memory_mb, pids, disk_mb, inodes, bandwidth_gb, limits jsonb, isolation(`runc`/`gvisor`) | |
| `feature_sets` / `feature_set_items` | id, name / feature_key, enabled | OpenPanel-এর features/*.txt |
| `modules` | name, enabled, version, installed_at | |
| `settings` | scope(`global`/`node`/`tenant`), key, value jsonb, updated_by | টাইপ করা config |
| `nodes` | id, name, hostname, public_ip, status, agent_version, cert_fingerprint, capacity jsonb, last_seen_at | |
| `node_certificates` | node_id, serial, not_after, revoked_at | mTLS |
| `ip_addresses` | id, node_id, ip, kind(`shared`/`dedicated`), tenant_id? | |
| `operations` | id, kind, tenant_id?, node_id, payload jsonb, status, attempts, idempotency_key, error, created_by, created_at, finished_at | operation ledger |
| `audit_logs` | id, actor_id, actor_ip, action, target_type, target_id, tenant_id?, node_id?, outcome, correlation_id, metadata jsonb, at | append-only (UPDATE/DELETE নিষেধ) |
| `notifications` | id, account_id, level, title, body, read_at, created_at | |
| `idempotency_keys` | key, account_id, request_hash, response_ref, expires_at | |
| `api_tokens` | id, account_id, name, hash, scopes[], expires_at, last_used_at | API ব্যবহারের জন্য |

## Web
| টেবিল | মূল field |
|---|---|
| `domains` | id, tenant_id, fqdn (unique), kind(`primary`/`addon`/`sub`/`alias`), docroot, php_version, status, suspended_at |
| `redirects` | id, domain_id, source_path, target, code |
| `dns_zones` / `dns_records` | zone_id, name, type, content, ttl, prio / dnssec |
| `dns_zone_templates` | id, name, records jsonb |
| `certificates` | id, domain_id, issuer, kind(`acme`/`custom`), not_after, status, pem_ref |
| `vhost_templates` | id, server(`nginx`/`apache`), name, body |
| `php_settings` | tenant_id, version, ini jsonb |
| `waf_policies` | scope, mode(`off`/`detect`/`block`), rules_disabled[] |

## Data
| টেবিল | মূল field |
|---|---|
| `databases` | id, tenant_id, engine(`mysql`/`postgres`), name, size_bytes |
| `database_users` / `database_grants` | | 
| `remote_db_hosts` | tenant_id, engine, cidr |
| `ftp_accounts` | id, tenant_id, username, home, quota_mb |
| `cron_jobs` | id, tenant_id, schedule, command, enabled, last_run_at, last_status |
| `backups` | id, tenant_id?, kind(`user`/`system`), destination_id, status, size, started_at, finished_at, manifest jsonb |
| `backup_destinations` | id, kind(`local`/`s3`/`sftp`), config_enc |
| `backup_schedules` | id, scope, cron, retention |
| `apps` | id, tenant_id, kind(`wordpress`/…), domain_id, version, path, status |
| `mail_domains` / `mailboxes` / `mail_aliases` | (email phase-এ) |

## Usage ও Monitoring
| টেবিল | মূল field |
|---|---|
| `usage_samples` | tenant_id, at, cpu_pct, mem_bytes, io_r, io_w, net_rx, net_tx | TimescaleDB বা partition (মাস অনুযায়ী) |
| `disk_usage` | tenant_id, at, bytes, inodes |

## License (Built Panel installation-এর দিকে)
| টেবিল | মূল field |
|---|---|
| `license_state` | installation_id, license_id, entitlement_jws, key_id, expires_at, last_refresh_at, mode(`active`/`grace`/`restricted`) |

License Authority-র নিজের DB আলাদা। PDF-এর ১৮টা টেবিল আছে [09-license-architecture.md](09-license-architecture.md)-এ।
