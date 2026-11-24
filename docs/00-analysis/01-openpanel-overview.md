# ০১ — OpenPanel সার্ভারের সার্বিক চিত্র

**সার্ভার:** `167.104.101.81` · hostname `builtpanel` · Linux 6.8.0-31-generic (Ubuntu 24.04)
**OpenPanel version:** `openpanel/openpanel:2.0.20`
**বিশ্লেষণের তারিখ:** ২০২৬-১১-১০ (install হয়েছে একই দিন সকালে)

সব তথ্য শুধু পড়ার কমান্ড দিয়ে নেওয়া। চিহ্নের অর্থ: **[দেখা]** মানে সরাসরি দেখা গেছে, **[অনুমান]** মানে নাম বা গঠন দেখে অনুমান করা।

## ১. চলমান অংশ

| অংশ | কোথায় চলে | পোর্ট | বিবরণ |
|---|---|---|---|
| **OpenAdmin** | host-এ systemd service (`admin.service`), **root** হিসেবে | 2087 | Go binary, ~৩২ MB, stripped। Admin panel। [দেখা] |
| **OpenPanel** | Podman container `openpanel` | 2083 | Go binary `/openpanel`। গ্রাহকের panel। [দেখা] |
| **Caddy** | container `caddy-coraza` | 80, 443, 2053; admin API 127.0.0.1:2019 | Reverse proxy, স্বয়ংক্রিয় SSL, Coraza WAF। [দেখা] |
| **MariaDB 10** | container `openpanel_mysql` | 127.0.0.1:3306 | Panel-এর নিজস্ব database। [দেখা] |
| **Redis 8.6** | container `openpanel_redis` | ভেতরের network | Cache আর session। [দেখা] |
| ClamAV, FTP, phpMyAdmin | compose-এ আছে, এখন চলছে না | — | চাহিদা অনুযায়ী চালু হয়। [দেখা] |
| **CSF** | host | — | Firewall (`/usr/local/csf`)। [দেখা] |
| **opencli** | host, `/usr/local/opencli` | — | Bash script-এর সংগ্রহ। Panel এগুলোকে ডেকে আসল কাজ করে। [দেখা] |

## ২. ফোল্ডার

```
/etc/openpanel/            ← সব config
  openpanel/conf/openpanel.config   (INI, ১৪টা section)
  openpanel/features/*.txt          (প্রতি plan-এ কোন module চালু)
  openpanel/custom_code/            (custom css/js/header/footer)
  caddy/ nginx/ apache/ openlitespeed/ openresty/ varnish/  (web server template)
  php/ mysql/ postgres/ bind9/ ftp/ clamav/ csf/ ofelia/ goaccess/ wordpress/ skeleton/
/usr/local/admin/          ← OpenAdmin binary + modules/security/csf.pl
/usr/local/opencli/        ← CLI (user/, domains/, email/, ftp/, plan/, php/, websites/, server/, lib/)
/root/docker-compose.yml   ← core service-এর compose file
```

## ৩. Architecture-এর ধরন

```
 ব্রাউজার ──► Caddy (443) ──► OpenAdmin (2087, host, root)  ──► opencli (bash) ──► podman / CSF / bind9 / OS
                         └──► OpenPanel (2083, container)   ──► opencli / user container
                                            │
                                MariaDB + Redis (core)
প্রত্যেক user ──► নিজের container stack (web server + PHP + DB ইত্যাদি)  [অনুমান]
```

**পর্যবেক্ষণ:**
1. **Admin panel root হিসেবে চলে।** PDF-এ ঠিক এটাই নিষেধ করা হয়েছে ("Do not run the public control plane as root")।
2. **আসল কাজ হয় bash script-এ।** Go API মূলত `opencli` ডাকে। এতে input validation দুর্বল হওয়ার ঝুঁকি থাকে।
3. **শুধু একটা server।** Cluster ফিচার (`/cluster/enroll`, `/cluster/fleet`) আছে, কিন্তু ভিত্তি single-node।
4. **Plan-এর feature set সাধারণ text file দিয়ে নিয়ন্ত্রিত।** এটা ভালো ধারণা। Built Panel-এ এটা database-এ রাখা হবে।
5. **Container runtime হলো Podman।** কোড Docker API-compatible, তাই Bollard দিয়ে দুটোই চালানো যাবে।

## ৪. Admin binary (REA বিশ্লেষণ)

| বিষয় | মান |
|---|---|
| Format | ELF 64-bit x86-64, PIE, stripped |
| Go version | go1.27.3 |
| Main module | `openadmin/cmd/openadmin` |
| Dependency | ৩৩টা |
| উল্লেখযোগ্য library | `golang-jwt/jwt`, `go-webauthn/webauthn` (passkey), `boombuler/barcode` (2FA QR), `gorilla/websocket` (terminal/live log), `gorilla/securecookie`, `robfig/cron`, `go-sql-driver/mysql`, `lib/pq`, `x/crypto/ssh`, `google/go-tpm`, `GehirnInc/crypt` |

এই library-গুলো থেকে বোঝা যায় admin-এ আছে JWT login, passkey (WebAuthn), TOTP 2FA, websocket terminal, cron scheduler, SSH (cluster/migration), আর TPM-ভিত্তিক license binding (সম্ভবত)।

API route-এর পূর্ণ তালিকা আছে [02-admin-panel-features.md](02-admin-panel-features.md)-এ।
