# ০১ — System Architecture

## মূল চিত্র

```
                       ┌───────────────────────────────────────────────┐
  Admin / Reseller /   │  React SPA (Vite)                             │
  Client ব্রাউজার ────►│  admin.html  │  client.html                   │
                       └──────────────┬────────────────────────────────┘
                                      │ HTTPS (session cookie + CSRF)
                       ┌──────────────▼────────────────────────────────┐
                       │  Edge: Caddy (TLS, rate limit, static SPA)    │
                       └──────────────┬────────────────────────────────┘
                                      │
   ┌──────────────────────────────────▼───────────────────────────────────┐
   │ CONTROL PLANE (Rust, একটা modular monolith, root নয়)                │
   │  control-api (Axum)   │  worker (Apalis jobs)   │  scheduler         │
   │  modules: auth, users, plans, domains, dns, ssl, … (প্রতিটা crate)   │
   │  PostgreSQL (ডেটা)  ·  Redis (cache/rate-limit/pubsub)               │
   └──────────────────────────────────┬───────────────────────────────────┘
                                      │ mTLS (node certificate) + signed typed ops
            ┌─────────────────────────┼─────────────────────────┐
   ┌────────▼────────┐       ┌────────▼────────┐       ┌────────▼────────┐
   │ Node 1 (VPS)    │       │ Node 2          │  …    │ Node N          │
   │ node-agent      │       │ node-agent      │       │ node-agent      │
   │  └ priv-helper  │       │                 │       │                 │
   │ Docker Engine   │       │                 │       │                 │
   │ Caddy (edge)    │       │                 │       │                 │
   │ tenant stacks:  │       │                 │       │                 │
   │  A[web,php] B…  │       │                 │       │                 │
   └─────────────────┘       └─────────────────┘       └─────────────────┘

   আলাদা: LICENSE AUTHORITY (Rust) ── শুধু Built Panel installation-এর license, release ও integrity
```

## Process ও বিশ্বাসের সীমানা (trust boundary)

| Process | User | কী করতে পারে | কী করতে পারে না |
|---|---|---|---|
| `control-api` | `builtpanel` (root নয়) | DB, Redis, agent-কে typed কাজ পাঠানো | Docker socket, host ফাইল |
| `worker` | `builtpanel` | Scheduled job, email, retry | একই |
| `node-agent` | `builtpanel-agent` (docker group অথবা rootless) | Bollard দিয়ে Docker, tenant volume, Caddy admin API | যেকোনো shell command |
| `priv-helper` | root, ছোট binary | শুধু নির্দিষ্ট কিছু কাজ (nftables rule, quota, user namespace) | তালিকার বাইরের কিছু |
| Tenant container | non-root UID | নিজের ফাইল আর নিজের service | অন্যের volume, Docker socket |

## Request-এর পথ (উদাহরণ: client domain যোগ করে)
1. React → `POST /api/v1/client/domains` (session + CSRF)।
2. `auth` middleware → `rbac` (permission `domains.create`) → `feature-gate` (plan-এ `domains` আছে? সীমা ছাড়িয়েছে?)।
3. `domains` module validation (FQDN, blocklist, আগে থেকে আছে কিনা) → DB transaction → `operations` টেবিলে `AttachDomain` (status `pending`) → audit log।
4. Worker operation টা নিয়ে সঠিক node-এর agent-এ পাঠায় (mTLS)।
5. Agent tenant আর node যাচাই করে → Caddy route + vhost ফাইল → ফলাফল পাঠায়।
6. Operation `succeeded` হয় → Redis pubsub → SSE দিয়ে UI আপডেট হয়।
7. ব্যর্থ হলে retry policy চলে, তারপর `failed` + notification।

## মূল নকশার নীতি
1. **Operation ledger:** প্রতিটা infra পরিবর্তন `operations` টেবিলে idempotency key সহ লেখা হয়। Agent-এ পাঠানো async। UI অবস্থা দেখায়।
2. **Desired state + reconcile:** DB-তে থাকে কাঙ্ক্ষিত অবস্থা। Agent-এর `Reconcile` কাজ পর্যায়ক্রমে বাস্তব অবস্থা মিলিয়ে নেয় (drift ঠিক করে)।
3. **Module boundary:** একটা module অন্য module-এর টেবিলে সরাসরি লিখবে না, public service trait ব্যবহার করবে।
4. **Fail-safe license:** License সমস্যা হলে শুধু panel management বন্ধ হবে। Website, DB আর mail চলতে থাকবে।
5. **Tenant scoping:** প্রতিটা query-তে `tenant_id` লাগবে। Repository স্তরেই এটা বাধ্যতামূলক।
