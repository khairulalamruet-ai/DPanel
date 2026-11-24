# ০২ — Admin Panel (OpenAdmin): ফিচার A থেকে Z

এই তালিকা বানানো হয়েছে admin binary-র ভেতরের route string থেকে, REA দিয়ে। Binary চালিয়ে দেখা হয়নি, তাই প্রতিটা ফিচার কতটা কাজ করে সেটা যাচাই করা নেই।
কলামের অর্থ: **Module** মানে Built Panel-এ যে module-এ এই ফিচার যাবে (দেখুন [../01-architecture/03-module-system.md](../01-architecture/03-module-system.md))।

## A. Account ও Admin
| ফিচার | OpenPanel route | Built Panel module |
|---|---|---|
| Admin login, whoami | `POST /api/login`, `GET /api/whoami` | `auth` |
| একাধিক admin, role | `/api/settings/administrators` | `auth` |
| Admin 2FA (TOTP) | `/api/security/2fa/enable`, `/disable` | `auth` |
| Passkey (WebAuthn) | `/api/security/passkeys` | `auth` |
| Admin panel বন্ধ করা | `/api/security/disable-admin` | `auth` |
| Basic auth (অতিরিক্ত স্তর) | `/api/security/basic-auth` | `auth` |
| Quickstart/tour | `/api/quickstart/dismiss`, `/api/tour/complete` | `ui-onboarding` |

## B. Backups
| ফিচার | Route | Module |
|---|---|---|
| User backup চালানো, settings, configuration | `/api/backups/user/{run,settings,configuration,action-status}` | `backups` |
| System backup চালানো, restore, delete | `/api/backups/system/{run,restore/{file},delete/{file},settings}` | `backups` |
| User export বানানো, download, delete | `/api/users/{u}/export/{create,status,download,delete}` | `backups` |

## C. Cluster
| ফিচার | Route/পেজ | Module |
|---|---|---|
| Node enroll, fleet, health | `/cluster/{enroll,fleet,health,sync,move,merge,routes,proxy}` | `nodes` |
| সব node-এ একসাথে প্রয়োগ | `/cluster/applyall` | `nodes` |
| DNS cluster | `/api/dns/cluster`, `/api/dns/cluster/{ip}` | `dns` |

## D. Dashboard ও Domains
| ফিচার | Route | Module |
|---|---|---|
| Dashboard summary, layout | `/dashboard/{summary,layout,plans}` | `dashboard` |
| Domain তালিকা, যোগ, bulk add | `/api/domains`, `/api/domains/new`, `/domains/bulk-add` | `domains` |
| Domain suspend, unsuspend, delete | `/domains/{suspend,unsuspend,delete}` | `domains` |
| VHost দেখা ও সম্পাদনা | `/api/domains/{d}/vhost/{u}` | `webserver` |
| VHost template, file template | `/api/domains/{vhost-templates,file-templates}` | `webserver` |
| Domain stats (GoAccess) | `/api/domains/{d}/stats/{u}` | `stats` |
| SSL, HSTS, rate limit, Cloudflare | `/domains/{ssl,hsts,ratelimit,cloudflare}` | `ssl`, `domains` |
| DNS zone template | `/api/dns/zone-templates` | `dns` |
| Docker info, tag | `/api/docker/info`, `/api/docker-tags` | `containers` |

## E. Emails
| ফিচার | Route | Module |
|---|---|---|
| Mail server install, settings | `/emails/install`, `/api/emails/settings` | `email` |
| Account তালিকা ও তৈরি | `/api/emails/accounts` | `email` |
| Domain অনুযায়ী সীমা | `/api/emails/domain-limits` | `email` |
| Mail queue | `/api/emails/queue` | `email` |
| Deliverability, report, webmail, import | `/emails/{deliverability,reports,webmail,import}` | `email` |

## F. Files ও FTP
| ফিচার | Route | Module |
|---|---|---|
| Admin file manager | `/filemanager` | `files` |
| FTP (vsftpd) পরিচালনা | `/ftp`, `/services/ftp` | `ftp` |
| Default skeleton file | `/api/settings/defaults/files[/{u}]` | `users` |

## I. Import ও Migration
| ফিচার | Route | Module |
|---|---|---|
| cPanel, CyberPanel বা OpenPanel থেকে import | `/api/import/{panel_type}`, `/import/{cpanel,cyberpanel,openpanel}` | `migration` |
| Backup upload, ফাইল তালিকা | `/api/import/upload`, `/api/import/backup-files` | `migration` |
| Transfer চলমান অবস্থা ও log | `/api/import/transfers[/{u}]`, `/api/import/logs/{account,transfer}/{file}` | `migration` |
| পুরো server migrate | `/api/server/migrate` | `migration` |

## L. License ও Logs
| ফিচার | Route | Module |
|---|---|---|
| License key, info, verify | `/api/license`, `/api/license/{info,verify}` | `license` |
| Log viewer (auth, caddy, panel, cron, syslog) | `/log/{auth,caddy,openpanel,openpanel-cron,syslog}` | `logs` |

## N. Notifications
| ফিচার | Route | Module |
|---|---|---|
| তালিকা, পড়া হয়েছে চিহ্ন, delete | `/api/notifications`, `/{line}/read` | `notifications` |
| Pause, resume, snooze | `/api/notifications/{pause,resume}`, `/notifications/snooze` | `notifications` |
| Notification settings | `/api/settings/notifications` | `notifications` |

## P. Plans, PHP, Processes
| ফিচার | Route | Module |
|---|---|---|
| Hosting plan CRUD | `/api/plans`, `/api/plans/{id}` | `plans` |
| Reseller package | `/api/plans/resellers` | `resellers` |
| Plan অনুযায়ী feature | `/api/settings/features[/{plan}]` | `plans` |
| PHP version, ini, options | `/api/settings/php`, `/php/{ini,options}` | `php` |
| Process তালিকা, kill | `/api/server/processes`, `/{pid}/{action}` | `server` |

## R. Resellers
| ফিচার | Route | Module |
|---|---|---|
| Reseller চালু বা বন্ধ, settings | `/api/settings/resellers[/enabled]` | `resellers` |
| Reseller activity, password, rename | `/resellers/{activity,password,rename,update}` | `resellers` |

## S. Security, Server, Services, Settings
| ফিচার | Route | Module |
|---|---|---|
| Firewall (CSF) | `/api/security/firewall` | `firewall` |
| WAF (Coraza) চালু করা, rule | `/api/security/waf`, `/api/security/waf/rules` | `waf` |
| User-agent blacklist | `/api/security/blacklist-useragents` | `waf` |
| Imunify, Rspamd, malware scan | `/security/{imunify,rspamd}` | `malware` |
| Server cron | `/api/server/crons` | `server` |
| Memory: cache drop, swap | `/api/server/memory/{drop-cache,drop-swap}`, `/api/server/swap` | `server` |
| Reboot, অবস্থা | `/api/server/reboot[/status]` | `server` |
| Root password | `/api/server/root-password` | `server` |
| SSH config, key | `/api/server/ssh[/config]` | `server` |
| Timezone | `/api/server/timezone` | `server` |
| Service তালিকা, অবস্থা, start/stop/restart | `/api/services`, `/api/service/{action}/{name}` | `services` |
| Podman image, network, volume, disk usage | `/api/services/podman/*` | `containers` |
| Image vulnerability scan, update check | `/api/services/podman/images/{vulnerabilities,check-update}` | `containers` |
| Crash log, service watcher | `/services/{crashlogs,watcher,monitored}` | `services` |
| General settings, branding, custom code | `/api/settings/{general,custom-code}` | `settings` |
| Module চালু বা বন্ধ | `/api/settings/modules` | `settings` |
| Locale (ভাষা) | `/api/settings/locales` | `i18n` |
| Caddy metrics | `/api/settings/caddy/metrics` | `monitoring` |
| Auto-update, update tag | `/api/settings/updates[/now,/tags]` | `updates` |
| Support report | `/api/support/report` | `support` |
| Web terminal | `/terminal` (websocket) | `terminal` |

## U. Users
| ফিচার | Route | Module |
|---|---|---|
| User CRUD | `GET/POST /api/users`, `GET/PUT/PATCH/DELETE /api/users/{u}` | `users` |
| Autologin (admin হিসেবে user-এর panel-এ ঢোকা) | `/api/users/{u}/autologin` | `users` |
| User-এর container, start/stop | `/api/users/{u}/containers[/{action}/{c}]` | `containers` |
| Permission reset | `/api/users/{u}/permissions/reset` | `users` |
| Suspend, unsuspend, rename, change plan | `/user/{suspend,unsuspend,edit}`, `/update/change_plan` | `users` |
| Resource usage (CPU, RAM, disk) | `/api/usage/{cpu,memory,disk,server}` | `monitoring` |
| IP তালিকা | `/api/ips` | `network` |

## W. Websites ও Webhooks
| ফিচার | Route | Module |
|---|---|---|
| সব website, PageSpeed | `/websites`, `/websites/pagespeed` | `websites` |
| Vulnerability ও malware scan | `/websites/{scan,vulnerabilities}` | `websites` |
| Git deploy | `/websites/git`, `/websites/repos` | `websites` |
| Webhook | `/api/webhooks/...` | `integrations` |

---
**মোট:** প্রায় ১৮০টা আলাদা API endpoint, যেগুলো Built Panel-এর ~৩০টা module-এ ভাগ হবে।
