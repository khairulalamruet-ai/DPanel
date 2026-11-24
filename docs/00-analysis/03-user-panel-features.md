# ০৩ — User Panel (OpenPanel): ফিচার A থেকে Z

**উৎস:** `/etc/openpanel/openpanel/features/default.txt` (৬০টা feature flag) আর `openpanel.config`-এর `enabled_modules`। [দেখা]
User panel-এর binary ভেতরে খোলা হয়নি, তাই প্রতিটা ফিচারের বিবরণ নাম দেখে লেখা। [অনুমান]

## Feature set (plan প্রিসেট)
OpenPanel-এ ৪টা প্রিসেট ফাইল আছে: `default.txt` (সব), `basic.txt`, `dns_only.txt`, `mysql_only.txt`।
**Built Panel-এ:** "Feature set" database-এর একটা entity হবে, আর প্রতিটা plan একটা feature set-এর সঙ্গে যুক্ত থাকবে।

## ফিচার তালিকা (module অনুযায়ী)

### অ্যাকাউন্ট ও নিরাপত্তা
| Flag | কাজ | Built Panel module |
|---|---|---|
| `account` | প্রোফাইল, ইমেইল, পাসওয়ার্ড | `account` |
| `sessions` | সক্রিয় session দেখা ও বন্ধ করা | `auth` |
| `twofa` | TOTP 2FA | `auth` |
| `login_history` | Login-এর ইতিহাস (IP, দেশ) | `auth` |
| `activity` | Activity log | `audit` |
| `locale` | ভাষা বেছে নেওয়া | `i18n` |
| `notifications` | User-এর notification | `notifications` |
| `favorites` | প্রিয় পেজ বা shortcut | `ui-core` |
| `summary` / `info` | Dashboard সারাংশ, অ্যাকাউন্টের তথ্য | `dashboard` |

### Domain, DNS, SSL
| Flag | কাজ | Module |
|---|---|---|
| `domains` | Domain যোগ, মুছে ফেলা, তালিকা | `domains` |
| `capitalize_domains` | Domain নাম দেখানোর ধরন | `domains` |
| `docroot` | Document root বদলানো | `domains` |
| `redirects` | 301/302 redirect | `domains` |
| `dns` | DNS zone editor | `dns` |
| `ssl` | Let's Encrypt বা custom SSL | `ssl` |
| `edit_vhost` | VHost সম্পাদনা | `webserver` |
| `webserver_conf` | Web server config | `webserver` |
| `varnish` | Varnish cache | `cache` |
| `temporary_links` | অস্থায়ী preview link | `domains` |

### Web server ও PHP
| Flag | কাজ | Module |
|---|---|---|
| `php` | PHP version বেছে নেওয়া | `php` |
| `php_options` / `php_ini` | PHP option, php.ini | `php` |
| `pm2` | Node.js app (PM2) | `apps-node` |
| `waf` | User পর্যায়ে WAF চালু বা বন্ধ | `waf` |

### Database
| Flag | কাজ | Module |
|---|---|---|
| `mysql` | MySQL/MariaDB database ও user | `db-mysql` |
| `remote_mysql` | Remote access-এর IP | `db-mysql` |
| `mysql_import` | SQL import | `db-mysql` |
| `mysql_conf` | MySQL config | `db-mysql` |
| `phpmyadmin` | phpMyAdmin SSO | `db-mysql` |
| `postgresql`, `remote_postgresql`, `postgresql_import`, `postgresql_conf` | PostgreSQL-এর জন্য একই ফিচার | `db-postgres` |
| `pgadmin` | pgAdmin SSO | `db-postgres` |

### Cache ও Search
| Flag | কাজ | Module |
|---|---|---|
| `redis` | Redis instance | `cache` |
| `memcached` | Memcached | `cache` |
| `elasticsearch` / `opensearch` | Search engine | `search-engines` |

### Files, FTP, Backup
| Flag | কাজ | Module |
|---|---|---|
| `filemanager` | File manager (upload, edit, archive) | `files` |
| `fix_permissions` | Permission ঠিক করা | `files` |
| `malware_scan` | ClamAV scan | `malware` |
| `disk_usage` / `inodes` | Disk ও inode ব্যবহার | `files` |
| `ftp` | FTP account | `ftp` |
| `backups` | User-এর নিজের backup ও restore | `backups` |

### Email
| Flag | কাজ | Module |
|---|---|---|
| `emails` | Mailbox, forwarder, autoresponder, webmail | `email` |

### Apps
| Flag | কাজ | Module |
|---|---|---|
| `wordpress` | WordPress install ও পরিচালনা | `apps-wordpress` |
| `website_builder` | Website builder | `apps-builder` |
| `autoinstaller` | এক ক্লিকে app install | `apps-installer` |
| `docker` | User-এর নিজের container | `containers` |

### Logs, Stats, Usage
| Flag | কাজ | Module |
|---|---|---|
| `domain_logs` | Access log | `logs` |
| `error_logs` | Error log | `logs` |
| `goaccess` | Visitor stats | `stats` |
| `usage` | CPU, RAM, I/O ইতিহাস | `monitoring` |
| `process_manager` | নিজের process দেখা ও kill | `processes` |
| `services` | নিজের service (nginx, php-fpm…) restart | `services` |
| `crons` | Cron job | `cron` |

## User panel-এর config (openpanel.config) থেকে পাওয়া settings
- **USERS:** login rate limit, session-এর মেয়াদ, password strength, weak password নিষেধ, 2FA বাধ্যতামূলক, onboarding।
- **PANEL:** auto-update, API চালু, template, demo mode, terminal timeout।
- **CAPTCHA:** reCAPTCHA, Turnstile।
- **AI:** AI assistant (provider, model, মাসিক সীমা)।
- **SMTP:** panel-এর mail পাঠানো।
- **DOCKER:** user-প্রতি সর্বোচ্চ RAM ও CPU।
- **FILES:** upload, edit ও download-এর সীমা, trash auto-purge।
- **DATABASES:** import size সীমা, সংরক্ষিত নাম, username prefix।
- **SECURITY:** IP-cookie যাচাই, user-agent blacklist।

Built Panel-এ এই সব settings একটা টাইপ করা `settings` টেবিলে যাবে (key, scope, value, schema)। INI ফাইল থাকবে না।
