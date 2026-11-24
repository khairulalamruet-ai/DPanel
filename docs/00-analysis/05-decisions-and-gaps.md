# ০৫ — সিদ্ধান্ত ও পার্থক্য (OpenPanel বনাম Built Panel)

## মূল সিদ্ধান্ত

| # | বিষয় | OpenPanel-এ | Built Panel-এ | কারণ |
|---|---|---|---|---|
| D1 | Backend ভাষা | Go | **Rust + Axum** | doc-guide PDF |
| D2 | Frontend | Go-র server-rendered template | **React + TS + Vite (SPA)** | আপনার নির্দেশ (Next.js নয়) |
| D3 | Panel DB | MariaDB | **PostgreSQL + SQLx** | PDF |
| D4 | কাজ করার উপায় | Bash `opencli` | **Typed Rust node agent** | PDF: "not a generic remote shell" |
| D5 | Admin process | root | **সাধারণ user**, privileged কাজ agent বা helper-এ | PDF: "Do not run the public control plane as root" |
| D6 | Server | একটা (cluster যোগ করা হয়েছে) | **শুরু থেকেই multi-node** (control plane + প্রতি server-এ agent) | PDF architecture |
| D7 | Agent সংযোগ | SSH | **mTLS**, node-প্রতি certificate | PDF |
| D8 | Feature set | `.txt` ফাইল | DB entity + module manifest | Module-based design |
| D9 | Config | INI ফাইল | টাইপ করা `settings` টেবিল + validation | Audit ও versioning |
| D10 | Container runtime | Podman | **Docker Engine** (Bollard)। Podman-এর compatible socket-এও চলবে | PDF |
| D11 | License | key + verify | **আলাদা License Authority service** (Ed25519, signed manifest, daily integrity) | License PDF |
| D12 | Audit | activity log ফাইল | append-only `audit_logs` টেবিল + correlation ID | PDF |

## Admin ও User panel: একটা app না দুটো?
**সিদ্ধান্ত:** একটা React codebase-এ দুটো "app shell" থাকবে: `admin` ও `client`। Router আর permission দিয়ে আলাদা করা হবে, build হবে দুটো আলাদা bundle (Vite multi-entry)। Backend একটাই control plane, route থাকবে `/api/v1/admin/*` আর `/api/v1/client/*`-এ।
কারণ: component, API client আর design system একবার লিখলেই হয়। আবার bundle আলাদা থাকায় client-এর কাছে admin কোড যায় না।

## User-এর database: আলাদা container না shared?
**সিদ্ধান্ত:** plan অনুযায়ী বেছে নেওয়া যাবে।
- `shared`: node-এ একটা MariaDB বা PostgreSQL, tenant-প্রতি আলাদা DB আর user। সস্তা।
- `dedicated`: tenant-এর নিজের DB container। ভালো isolation, বেশি RAM লাগে।
প্রথম রিলিজে শুধু `shared` থাকবে, `dedicated` পরের phase-এ।

## নিরাপত্তা নিয়ে PDF-এর বাধ্যতামূলক বিষয়
- Customer container non-root, `cap_drop: ALL`, seccomp/AppArmor, pids/cpu/mem/storage limit।
- Docker socket কখনো customer বা public API-র নাগালে যাবে না।
- শুধু একটা container-কে নিরাপত্তার সীমানা ধরা যাবে না। উচ্চ ঝুঁকির plan-এ gVisor বা Kata বিবেচনা করা হবে।
- License enforcement-এ **কখনো** গ্রাহকের ডেটা মুছে ফেলা বা website বন্ধ করা যাবে না। শুধু panel management সীমিত হবে (restricted mode)।

## প্রথম রিলিজে (MVP) কী থাকবে, কী পরে

| MVP (v1) | পরে (v2+) |
|---|---|
| Auth, 2FA, RBAC, audit | Passkey (WebAuthn) |
| Node agent + ১টা node | Multi-node scheduling, tenant migration |
| Users, plans, feature set, resource limit | Reseller |
| Domains, DNS (PowerDNS), SSL (ACME) | DNSSEC, DNS cluster, Cloudflare |
| nginx + PHP-FPM, version বেছে নেওয়া | Apache, OpenLiteSpeed, Varnish |
| MySQL (shared), phpMyAdmin SSO | PostgreSQL, dedicated DB, pgAdmin |
| File manager, SFTP/FTP | Git deploy |
| Backup ও restore (local + S3) | Incremental, remote destination |
| Cron, log, usage chart | GoAccess, PageSpeed |
| WAF (Coraza), firewall (nftables), ClamAV | Imunify, Rspamd |
| WordPress installer | Website builder, autoinstaller catalog, Node/PM2 |
| License client + License Authority (basic) | Daily integrity scan, release signing pipeline |
| — | Email stack (জটিল, তাই আলাদা phase), cPanel/CyberPanel import, AI assistant |

## বাদ দেওয়া বা পরিবর্তন করা বিষয়
- **Root password বদলানো**, **web terminal** (root shell): ঝুঁকিপূর্ণ। থাকলেও admin-only থাকবে, 2FA আবার যাচাই করতে হবে, আর audit হবে। MVP-তে থাকবে না।
- **Demo mode**: পরে।
- **AI assistant**: পরে, ঐচ্ছিক module হিসেবে।
