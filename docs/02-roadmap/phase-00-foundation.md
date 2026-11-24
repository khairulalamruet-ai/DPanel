# Phase 00 — Foundation

**লক্ষ্য:** খালি কিন্তু সম্পূর্ণ চালু কঙ্কাল: Rust workspace, React workspace, DB, CI আর local dev environment।

### T00.01 — Repo ও নিয়ম
- **কী করতে হবে:** git repo, `.editorconfig`, `.gitignore`, `CONTRIBUTING.md`, commit নিয়ম (Conventional Commits), branch নিয়ম, `LICENSE` (নিজের), `CODEOWNERS`।
- **ফলাফল:** repo root
- **DoD:** README-তে প্রজেক্টের পরিচয় আর dev শুরু করার ধাপ আছে।

### T00.02 — Rust workspace কঙ্কাল
- **কী করতে হবে:** `Cargo.toml` workspace। খালি `apps/{control-api,worker,node-agent,priv-helper,bpctl}`, `crates/{kernel,common,domain,database,http-kit,observability}`। `rust-toolchain.toml` (stable, pinned)।
- **DoD:** `cargo build --workspace` সফল।

### T00.03 — Observability init
- **কী করতে হবে:** `observability` crate-এ tracing subscriber (JSON log), request-id, OTel exporter (env দিয়ে চালু বা বন্ধ), Prometheus `/metrics`।
- **DoD:** control-api চালু করলে JSON log আসে, আর `/metrics`-এ HTTP counter দেখা যায়।

### T00.04 — Config লোডার
- **কী করতে হবে:** `figment` দিয়ে `control.toml` + `BUILTPANEL_*` env। Secret ফাইল থেকে পড়া। শুরুতে validation।
- **DoD:** ভুল config দিলে পরিষ্কার error দিয়ে বন্ধ হয়।

### T00.05 — Error ও response মান
- **কী করতে হবে:** `http-kit`-এ `AppError` → Problem Details JSON। `correlation_id` যুক্ত করা।
- **DoD:** test-এ 400, 401, 403, 404, 409, 422 আর 500-এর সঠিক JSON আসে।

### T00.06 — Axum app + health
- **কী করতে হবে:** control-api-তে router, tower-http layer (trace, timeout, cors, compression, body limit), `GET /api/v1/public/health` (liveness), `/ready` (DB + Redis)।
- **DoD:** `curl /health` 200 দেয়। DB বন্ধ থাকলে `/ready` 503 দেয়।

### T00.07 — PostgreSQL + SQLx
- **কী করতে হবে:** `database` crate-এ pool, transaction helper, `sqlx migrate`। প্রথম migration-এ `schema_info` আর UUID v7 function বা extension।
- **DoD:** `bpctl migrate` চলে। `sqlx::test` দিয়ে একটা test pass করে।

### T00.08 — Redis
- **কী করতে হবে:** Redis client, health check, `/ready`-এ যুক্ত করা।
- **DoD:** Redis বন্ধ থাকলে `/ready` 503 দেয়।

### T00.09 — OpenAPI
- **কী করতে হবে:** `utoipa` setup, `/api/v1/openapi.json`, dev-এ Swagger/Scalar UI।
- **DoD:** health endpoint spec-এ দেখা যায়।

### T00.10 — Local dev environment
- **কী করতে হবে:** `deploy/dev/compose.yml` (Postgres, Redis, Caddy, Mailpit), `justfile` বা `Makefile` কমান্ড (`dev`, `test`, `migrate`, `fmt`)।
- **DoD:** নতুন developer ৩টা কমান্ডে সব চালু করতে পারেন।

### T00.11 — Frontend workspace
- **কী করতে হবে:** `web/` pnpm workspace। `apps/admin` আর `apps/client` (Vite + React + TS strict)। `packages/{ui,api,auth,i18n,shell}` খালি।
- **DoD:** `pnpm dev:admin` আর `pnpm dev:client` চালু হয়। Vite proxy `/api` → control-api।

### T00.12 — UI ভিত্তি
- **কী করতে হবে:** Tailwind + shadcn/ui init, theme token (light/dark), বাংলা font, `packages/ui`-তে Button, Input, Card, Dialog, Table, Toast।
- **DoD:** একটা demo পেজে সব component দুটো theme-এই ঠিকভাবে দেখা যায়।

### T00.13 — Router ও Query
- **কী করতে হবে:** TanStack Router (file-based) + TanStack Query provider। ৪০৪ আর error boundary।
- **DoD:** `/` আর একটা ভুল URL দুটোই সঠিক পেজ দেখায়।

### T00.14 — API type generation
- **কী করতে হবে:** `openapi-typescript` script → `packages/api/src/schema.d.ts`। `openapi-fetch` client, CSRF header interceptor।
- **DoD:** health API টাইপসহ কল হয়।

### T00.15 — i18n
- **কী করতে হবে:** i18next, `bn` আর `en`, ভাষা বদলানোর বোতাম। তারিখ ও সংখ্যার format।
- **DoD:** ভাষা বদলালে পুরো UI বদলায়।

### T00.16 — CI
- **কী করতে হবে:** GitHub Actions: Rust (fmt, clippy, test, cargo-deny, cargo-audit), web (lint, typecheck, test, build), SQLx offline check।
- **DoD:** PR-এ সব check সবুজ দেখায়।

### T00.17 — ADR
- **কী করতে হবে:** `docs/adr/`-এ প্রথম ADR: (১) Rust/Axum, (২) React/Vite (Next.js নয়), (৩) PostgreSQL, (৪) agent protocol।
- **DoD:** ৪টা ADR merge হয়েছে।
