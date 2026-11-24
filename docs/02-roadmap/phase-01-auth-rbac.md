# Phase 01 — Auth, Session, 2FA, RBAC, Audit

**লক্ষ্য:** নিরাপদ login, role ও permission, আর প্রতিটা কাজের audit।

### T01.01 — accounts migration
- **কী করতে হবে:** `accounts`, `sessions`, `login_events` টেবিল ([06](../01-architecture/06-database-schema.md))।
- **DoD:** Migration up আর down দুটোই চলে।

### T01.02 — Password hashing
- **কী করতে হবে:** `auth-core`-এ argon2id hash ও verify, parameter config থেকে। Weak password list যাচাই।
- **DoD:** Unit test আছে। Hash হতে ~২০০ms লাগে (benchmark দিয়ে দেখা)।

### T01.03 — Bootstrap admin
- **কী করতে হবে:** `bpctl admin create` → প্রথম super_admin। পাসওয়ার্ড প্রম্পটে দিতে হবে, log-এ কখনো আসবে না।
- **DoD:** কমান্ড চালানোর পর DB-তে admin থাকে।

### T01.04 — Session store
- **কী করতে হবে:** Redis-এ session (random 256-bit), idle ও absolute timeout, cookie `__Host-bp_session; HttpOnly; Secure; SameSite=Strict`।
- **DoD:** Test-এ দেখা যায় মেয়াদ শেষে 401 আসে।

### T01.05 — Login/logout API
- **কী করতে হবে:** `POST /public/login`, `POST /logout`, `GET /me`। Login-এর পর session rotate হয়। `login_events` লেখা হয়।
- **DoD:** ভুল password দিলে সাধারণ ত্রুটি দেখায়, user আছে কিনা ফাঁস করে না।

### T01.06 — Rate limit ও lockout
- **কী করতে হবে:** IP আর username অনুযায়ী Redis sliding window। ৫ বার ব্যর্থ হলে ১৫ মিনিট lockout (configurable)।
- **DoD:** Test-এ ৬ষ্ঠ চেষ্টায় 429 আসে।

### T01.07 — CSRF
- **কী করতে হবে:** Session-বাঁধা CSRF token। সব state-changing request-এ `X-CSRF-Token` যাচাই।
- **DoD:** Header ছাড়া POST দিলে 403 আসে।

### T01.08 — TOTP 2FA
- **কী করতে হবে:** `account_mfa`। Enroll করা (QR + secret), confirm করা, ১০টা backup code, disable করা (password দিয়ে)। Login-এর সময় দ্বিতীয় ধাপ।
- **DoD:** 2FA চালু account দ্বিতীয় ধাপ ছাড়া login করতে পারে না।

### T01.09 — Sudo mode
- **কী করতে হবে:** Sensitive endpoint-এ `require_recent_auth(10m)` extractor।
- **DoD:** ১০ মিনিট পরে আবার যাচাই চাওয়া হয়।

### T01.10 — Session তালিকা ও revoke
- **কী করতে হবে:** `GET /sessions`, `DELETE /sessions/{id}`, "সব জায়গা থেকে logout"।
- **DoD:** Revoke করা session দিয়ে আর কোনো request চলে না।

### T01.11 — RBAC model
- **কী করতে হবে:** `roles`, `role_permissions`, `account_roles`। Built-in role seed করা। `rbac::require(perm)` extractor।
- **DoD:** Permission না থাকলে 403 আসে। Unit ও integration test আছে।

### T01.12 — Audit writer
- **কী করতে হবে:** `audit` crate। `audit_logs` টেবিল। App DB role-এ শুধু INSERT অধিকার। Middleware correlation_id আর actor_ip যোগ করে।
- **DoD:** Login, logout আর failed login audit-এ দেখা যায়। UPDATE চেষ্টা করলে DB error দেয়।

### T01.13 — Admin management API
- **কী করতে হবে:** `/admin/administrators` CRUD + role assign + 2FA বাধ্যতামূলক flag।
- **DoD:** `super_admin` ছাড়া কেউ অন্য admin বানাতে পারে না।

### T01.14 — Login UI (admin + client)
- **কী করতে হবে:** Login পেজ, 2FA ধাপ, ত্রুটির বার্তা, "ভাষা" বোতাম।
- **DoD:** Playwright test: login → dashboard → logout।

### T01.15 — Account security UI
- **কী করতে হবে:** Password বদলানো, 2FA enroll, backup code, session তালিকা, login ইতিহাস।
- **DoD:** UI থেকে সব কাজ করা যায়।

### T01.16 — Audit viewer UI (admin)
- **কী করতে হবে:** Filter (actor, action, তারিখ, tenant), pagination, JSON metadata দেখা।
- **DoD:** ১০ হাজার row-এও দ্রুত চলে (index যাচাই করা)।
