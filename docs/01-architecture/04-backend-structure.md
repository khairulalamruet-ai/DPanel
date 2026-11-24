# ০৪ — Backend (Rust workspace) গঠন

PDF-এর প্রস্তাবিত গঠন আর module system মিলিয়ে বানানো। Clean Architecture + DDD। Domain নিয়ম Axum, SQLx বা Redis-এর ওপর নির্ভর করবে না।

```
builtpanel/
├── Cargo.toml                    # workspace
├── apps/
│   ├── control-api/              # Axum HTTP server (admin + client API)
│   ├── worker/                   # Apalis job runner + scheduler
│   ├── node-agent/               # প্রতি server-এ (Bollard)
│   ├── priv-helper/              # ছোট root helper (allowlist করা কাজ)
│   ├── license-authority/        # আলাদা service (পরের phase)
│   └── bpctl/                    # admin CLI (bootstrap, recovery)
├── crates/
│   ├── kernel/                   # module trait, registry, event bus, error type
│   ├── common/                   # id type, time, pagination, result
│   ├── domain/                   # shared entity ও value object (Fqdn, Username, Quota…)
│   ├── database/                 # sqlx pool, transaction helper, tenant-scoped repo base
│   ├── auth-core/                # session, password, totp, CSRF
│   ├── rbac/                     # role, permission, policy check
│   ├── feature-gate/             # module + plan feature + limit check
│   ├── audit/                    # append-only audit writer
│   ├── node-protocol/            # agent-এর typed request/response (serde), version
│   ├── docker-manager/           # Bollard-এর ওপর সীমিত API (শুধু agent ব্যবহার করে)
│   ├── crypto/                   # ed25519, sha256, key id, canonical JSON
│   ├── observability/            # tracing, otel, metrics init
│   ├── http-kit/                 # Axum extractor, error→response, OpenAPI helper
│   └── modules/
│       ├── users/ plans/ tenants/ settings/ notifications/ nodes/ operations/
│       ├── domains/ dns/ ssl/ webserver/ php/ cache/ waf/
│       ├── db-mysql/ db-postgres/ files/ ftp/ backups/ malware/ email/
│       ├── apps-wordpress/ apps-installer/ containers/
│       └── services/ processes/ cron/ logs/ stats/ monitoring/ firewall/ updates/ migration/ resellers/
├── migrations/                   # কেবল kernel/core-এর migration (module-এর migration নিজের crate-এ)
├── modules/                      # module.toml manifest
├── deploy/{systemd,docker,caddy,monitoring}/
├── tests/{integration,security,e2e,fixtures}/
└── docs/
```

## প্রতিটা module crate-এর ভেতরের গঠন

```
crates/modules/domains/
├── Cargo.toml
├── migrations/
└── src/
    ├── lib.rs            # Module trait বাস্তবায়ন, registry-তে export
    ├── domain/           # entity, rule, error (Axum বা SQLx ছাড়া)
    ├── application/      # use-case: AddDomain, RemoveDomain… (trait-নির্ভর)
    ├── infrastructure/   # sqlx repo, agent client adapter
    ├── http/
    │   ├── admin.rs      # admin route
    │   ├── client.rs     # client route
    │   └── dto.rs        # request/response + utoipa schema
    ├── jobs.rs           # worker job
    └── events.rs         # publish ও subscribe
```

## API-র নিয়ম
- Base: `/api/v1/admin/...`, `/api/v1/client/...`, `/api/v1/reseller/...`, `/api/v1/public/...` (login ও health)।
- Error format (RFC 9457 Problem Details): `{ type, title, status, detail, code, correlation_id, errors[] }`।
- Pagination: `?cursor=&limit=`, response `{ items, next_cursor }`।
- দীর্ঘ কাজ: `202 Accepted` + `{ operation_id }`। অবস্থা দেখার জন্য `GET /operations/{id}` বা SSE `/events`।
- Idempotency: সব POST-এ `Idempotency-Key` header সমর্থিত।
- OpenAPI: `/api/v1/openapi.json` (utoipa)। Frontend-এর type এখান থেকে তৈরি হবে।
- Versioning: ভাঙনমূলক পরিবর্তন হলে `/api/v2`।

## Configuration
- `/etc/builtpanel/control.toml` + env override (`BUILTPANEL_*`)।
- Secret (DB password, session key, CA key) থাকবে `/etc/builtpanel/secrets/` (0600)-এ বা systemd credential-এ। Source code-এ কখনো নয়।
