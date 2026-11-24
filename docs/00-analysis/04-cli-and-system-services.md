# ০৪ — opencli ও system service

OpenPanel-এ আসল কাজ (container বানানো, domain যোগ ইত্যাদি) হয় `/usr/local/opencli/`-এর bash script দিয়ে। এখান থেকে বোঝা যায় Built Panel-এর **node agent**-কে কোন কোন **typed operation** দিতে হবে।

> PDF-এর নির্দেশ: "Do not make the node agent a generic remote shell. Expose typed operations…"
> তাই Built Panel-এ bash script থাকবে না। প্রতিটা কাজ হবে Rust-এ লেখা একটা typed operation।

## Script থেকে agent operation

| opencli script | কাজ | Built Panel agent operation (প্রস্তাব) |
|---|---|---|
| `user/add.sh` | User আর তার container stack বানানো | `CreateTenant`, `ProvisionTenantStack` |
| `user/delete.sh` | User মুছে ফেলা | `DeleteTenant` (soft delete + গ্রেস পিরিয়ড) |
| `user/suspend.sh` / `unsuspend.sh` | Suspend করা | `SuspendTenant`, `ResumeTenant` |
| `user/quota.sh` | Disk quota | `SetStorageQuota` |
| `user/change_plan.sh` | CPU, RAM ও limit বদলানো | `SetResourceLimits` |
| `user/rename.sh` | Username বদলানো | `RenameTenant` |
| `user/password.sh`, `2fa.sh`, `login.sh` | Auth | control plane-এ হবে, agent-এ নয় |
| `user/backup.sh`, `restore.sh` | Backup | `CreateBackup`, `RestoreBackup` |
| `user/transfer.sh` | অন্য server-এ সরানো | `ExportTenant` + `ImportTenant` |
| `user/block_ip.sh` | IP block | `AddFirewallRule` |
| `user/ip.sh` | Dedicated IP | `AssignIp` |
| `user/varnish.sh`, `litespeed.sh` | Stack-এর অংশ বদলানো | `SetStackComponent` |
| `domains/add.sh`, `delete.sh`, `edit.sh` | Domain | `AttachDomain`, `DetachDomain` |
| `domains/ssl.sh` | SSL | `IssueCertificate`, `InstallCustomCertificate` |
| `domains/dns.sh`, `dnssec.sh`, `update_ns.sh` | DNS | `UpsertZone`, `SignZone` |
| `domains/docroot.sh` | Docroot | `SetDocroot` |
| `domains/hsts.sh`, `ratelimit.sh` | Header ও limit | `SetVhostPolicy` |
| `domains/suspend.sh` | Domain suspend | `SuspendDomain` |
| `domains/stats.sh` | GoAccess | `GenerateDomainStats` |
| `domains/cloudflare.sh` | Cloudflare | control plane-এ integration |
| `email/setup.sh`, `server.sh` | Mail server | `ProvisionMailServer` |
| `email/manage.sh`, `quotas.sh` | Mailbox | `UpsertMailbox`, `DeleteMailbox` |
| `email/ratelimit.sh` | পাঠানোর সীমা | `SetMailRateLimit` |
| `email/webmail.sh` | Webmail | `ProvisionWebmail` |
| `ftp/add.sh` … `quota.sh` | FTP | `UpsertFtpAccount`, `DeleteFtpAccount` |
| `php/default.sh`, `domain.sh` | PHP version | `SetPhpVersion` |
| `plan/apply.sh` | সব user-এ plan প্রয়োগ | control plane loop + `SetResourceLimits` |
| `files/fix_permissions.sh` | Permission | `FixPermissions` |
| `files/malware_scan.sh` | ClamAV | `ScanPath` |
| `files/purge_trash.sh` | Trash | `PurgeTrash` |
| `websites/scan.sh`, `vulnerability.sh` | Scan | `ScanWebsite` |
| `websites/pagespeed.sh` | PageSpeed | control plane (বাইরের API) |
| `docker/collect_stats.sh` | Usage সংগ্রহ | `CollectMetrics` (stream) |
| `docker/logs.sh` | Container log | `StreamLogs` |
| `docker/update.sh` | Image update | `PullImage`, `RecreateContainer` |
| `server/migrate.sh` | Server migration | `ExportNode` (পরের phase) |
| `server/logrotate.sh` | Log rotate | agent-এর ভেতরের job |
| `backup.sh` | System backup | `CreateSystemBackup` |
| `waf.sh` | WAF | `SetWafPolicy` |
| `proxy.sh`, `port.sh` | Proxy ও port | `UpsertProxyRoute` |
| `sentinel.sh` | Health monitoring ও alert | agent health + control plane alert |
| `license.sh` | License | `license` module (আলাদা service) |
| `update.sh`, `version.sh` | Panel update | signed release + agent self-update |
| `imunify.sh` | Imunify360 | ভবিষ্যতের integration |

## System service (host-এ বা container-এ)

| Service | OpenPanel-এ | Built Panel-এ প্রস্তাব |
|---|---|---|
| Reverse proxy ও SSL | Caddy + Coraza WAF | **Caddy + Coraza** (Caddy admin API agent থেকে নিয়ন্ত্রণ) |
| Web server (user) | nginx, apache, OpenLiteSpeed, OpenResty | প্রথমে nginx + apache, পরে OLS |
| PHP | একাধিক version | PHP-FPM image, version অনুযায়ী |
| Database (panel) | MariaDB | **PostgreSQL** (PDF অনুযায়ী) |
| Database (user) | MySQL/MariaDB, PostgreSQL | একই। User-প্রতি container বা shared instance (সিদ্ধান্ত [05](05-decisions-and-gaps.md)-এ) |
| Cache | Redis, Memcached, Varnish | একই |
| DNS | BIND9 | **PowerDNS** (API আছে, cluster সহজ) বা BIND9 |
| Mail | Postfix/Dovecot (container) | Mail stack container (যেমন docker-mailserver) |
| FTP | vsftpd/pure-ftpd | Pure-FTPd বা SFTP-only |
| Firewall | CSF | nftables (agent নিয়ন্ত্রণ করবে) |
| Antivirus | ClamAV | ClamAV |
| Cron (container) | Ofelia | agent-এর নিজস্ব scheduler |
| Stats | GoAccess | GoAccess |
