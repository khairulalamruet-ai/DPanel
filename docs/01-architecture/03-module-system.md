# ০৩ — Module System

Built Panel সম্পূর্ণ module-ভিত্তিক হবে, ধারণাটা OpenPanel-এর `enabled_modules` আর `features/*.txt` থেকে নেওয়া। পার্থক্য হলো এখানে প্রতিটা module backend, frontend আর agent তিন জায়গাতেই একটা স্পষ্ট একক।

## Module কী
একটা module = নিচের অংশগুলোর একটা গুচ্ছ:

| অংশ | কোথায় | বাধ্যতামূলক? |
|---|---|---|
| **Manifest** (`module.toml`) | `modules/<name>/` | হ্যাঁ |
| Backend crate | `crates/modules/<name>/` | হ্যাঁ |
| DB migration | crate-এর `migrations/` | দরকার হলে |
| Frontend feature | `web/src/modules/<name>/` | UI থাকলে |
| Agent operation | `crates/node-protocol` + `apps/node-agent/src/ops/<name>.rs` | infra কাজ থাকলে |
| Translation | `web/src/modules/<name>/locales/{bn,en}.json` | UI থাকলে |

## Manifest (উদাহরণ গঠন, কোড নয়)

```toml
[module]
name        = "domains"
version     = "1.0.0"
title       = { en = "Domains", bn = "ডোমেইন" }
category    = "web"               # web | db | mail | files | security | system | apps
core        = false               # true হলে বন্ধ করা যাবে না (auth, users…)
depends_on  = ["webserver", "dns"]

[permissions]                     # RBAC permission key
keys = ["domains.read", "domains.create", "domains.update", "domains.delete"]

[features]                        # plan/feature-set-এ চালু বা বন্ধ করার key
keys = ["domains", "redirects", "docroot", "temporary_links"]

[limits]                          # plan-এ সংখ্যাগত সীমা
max_domains     = { type = "int", default = 10, unlimited = -1 }
max_subdomains  = { type = "int", default = 20 }

[ui]
admin_nav  = { group = "Web", order = 10, icon = "globe" }
client_nav = { group = "Domains", order = 10, icon = "globe" }

[agent]
operations = ["AttachDomain", "DetachDomain", "SetDocroot", "SetVhostPolicy"]
```

## Backend-এ module-এর চুক্তি (trait — ধারণা)
প্রতিটা module crate একটা `Module` বাস্তবায়ন export করবে, যেটা দেয়:
- `manifest()`: উপরের তথ্য।
- `routes(state)`: `admin_router` আর `client_router` (Axum `Router`)।
- `migrations()`: নিজের migration।
- `jobs()`: worker job registration।
- `on_event(event)`: অন্য module-এর event-এ সাড়া দেওয়া (যেমন `TenantDeleted` হলে domain মুছে ফেলা)।
- `health()`: module-এর health।

`apps/control-api` চালু হওয়ার সময় **registry** সব module লোড করে। তারপর DB-র `modules` টেবিল দেখে বন্ধ module-এর route mount করে না। Dependency check-ও এখানেই হয়।

> ডিফল্ট প্রস্তাব হলো **compile-time module** (Cargo feature / workspace crate)। Runtime plugin (dylib বা WASM) পরে বিবেচনা করা হবে, কারণ এতে নিরাপত্তা আর ABI-র জটিলতা অনেক বেশি।

## Frontend-এ module
- `web/src/modules/<name>/index.ts` থেকে export হয়: `routes`, `navItems`, `permissions`, `featureKey`।
- `web/src/app/module-registry.ts` সব module একত্র করে।
- Login-এর পর `GET /api/v1/{admin|client}/bootstrap` থেকে আসে: চালু module, user-এর permission আর plan feature। এগুলো দিয়ে menu আর route ফিল্টার হয়।
- Route-guard: feature বন্ধ থাকলে 404 না দেখিয়ে "আপনার plan-এ এই ফিচার নেই" পেজ দেখানো হয়।

## তিন স্তরের gate
একটা কাজ হতে হলে **তিনটাই** সত্য হতে হবে:
1. **Module enabled** (admin সারা server-এর জন্য চালু করেছেন)।
2. **Feature in plan** (user-এর plan-এর feature set-এ আছে)।
3. **Permission in role** (user-এর role-এ permission আছে)।
এর পর আসে **limit check** (সংখ্যাগত সীমা)। এটা transaction-এর ভেতরে হবে, যাতে race condition না হয়।

## Module তালিকা (প্রস্তাব)

| Category | Module | Core? |
|---|---|---|
| core | `auth`, `rbac`, `users`, `tenants`, `plans`, `settings`, `audit`, `notifications`, `nodes`, `operations`, `license-client`, `i18n`, `dashboard` | হ্যাঁ |
| web | `domains`, `dns`, `ssl`, `webserver`, `php`, `cache`, `waf` | না |
| db | `db-mysql`, `db-postgres` | না |
| files | `files`, `ftp`, `backups`, `malware` | না |
| mail | `email` | না |
| apps | `apps-wordpress`, `apps-installer`, `apps-node`, `containers` | না |
| system | `services`, `processes`, `cron`, `logs`, `stats`, `monitoring`, `firewall`, `updates`, `migration`, `resellers`, `integrations`, `support` | না |
