# Built Panel — পরিকল্পনা ও বিশ্লেষণ ডকুমেন্টেশন

Built Panel হলো Docker/Podman-ভিত্তিক web hosting control panel। প্রত্যেক গ্রাহক নিজের আলাদা container পান, আর একটা panel দিয়ে অনেকগুলো VPS বা dedicated server পরিচালনা করা যায়।

এই ফোল্ডারে এখনো কোনো কোড নেই। আছে তিনটা জিনিস:
1. একটা চলমান OpenPanel সার্ভারের **ফিচার বিশ্লেষণ** (reference হিসেবে)।
2. Built Panel-এর **architecture**।
3. ছোট ছোট task-এ ভাগ করা **ধাপে ধাপে roadmap**।

> **লক্ষ্য: Feature parity।** OpenPanel-এর সব ফিচার Built Panel-এ একই রকম থাকবে (তালিকা: `00-analysis/`)। কিন্তু code নিজেদের লেখা হবে।

> All features will be add Go to rust Language (Backend)

## প্রযুক্তি (সংক্ষেপে)

`doc-guide/` ফোল্ডারের দুটো PDF অনুসরণ করা হয়েছে, শুধু একটা পরিবর্তন আছে: **Frontend Next.js-এর বদলে React (Vite)**।

| স্তর | প্রযুক্তি |
|---|---|
| Backend API | Rust + Axum + Tokio + Tower |
| Node agent | Rust (আলাদা process, mTLS) |
| Container | Bollard (Docker API; Podman-এর Docker-compatible socket-এও চলে) |
| Database | PostgreSQL + SQLx |
| Cache/coordination | Redis |
| Jobs | Apalis / durable queue |
| Crypto | Ed25519 (signature), SHA-256 (integrity), Argon2id (password) |
| Observability | tracing + OpenTelemetry + Prometheus + Grafana |
| Frontend | **React + TypeScript + Vite** (Next.js নয়) |
| Deploy | Linux + systemd + Docker Engine |

বিস্তারিত আছে [01-architecture/02-tech-stack.md](01-architecture/02-tech-stack.md)-এ।

## ফোল্ডারের গঠন

```
docs/
├── README.md                     ← এই ফাইল
├── 00-analysis/                  ← OpenPanel থেকে কী শিখলাম
│   ├── 01-openpanel-overview.md
│   ├── 02-admin-panel-features.md
│   ├── 03-user-panel-features.md
│   ├── 04-cli-and-system-services.md
│   └── 05-decisions-and-gaps.md
├── 01-architecture/              ← Built Panel কীভাবে বানাব
│   ├── 01-system-architecture.md
│   ├── 02-tech-stack.md
│   ├── 03-module-system.md
│   ├── 04-backend-structure.md
│   ├── 05-frontend-structure.md
│   ├── 06-database-schema.md
│   ├── 07-security-model.md
│   ├── 08-node-agent-protocol.md
│   └── 09-license-architecture.md
└── 02-roadmap/                   ← ধাপে ধাপে কাজ
    ├── README.md                 ← phase তালিকা, ক্রম, task-এর নিয়ম
    ├── phase-00-foundation.md
    ├── phase-01-auth-rbac.md
    ├── …
    └── phase-19-hardening-release.md
```

## কীভাবে পড়বেন

1. প্রথমে `00-analysis/05-decisions-and-gaps.md` পড়ুন। কী বানাব আর কী বাদ দেব, সেটা এখানে আছে।
2. তারপর `01-architecture/01-system-architecture.md` আর `03-module-system.md`।
3. কাজ শুরু করবেন `02-roadmap/README.md` থেকে। প্রতিটা phase-এর task ক্রম অনুযায়ী করতে হবে।

## তথ্যের উৎস

- সার্ভার `167.104.101.81` (hostname `builtpanel`), OpenPanel 2.0.20। বিশ্লেষণ হয়েছে ২০২৬-১১-১০ তারিখে, শুধু পড়ার কমান্ড দিয়ে।
- Admin binary `/usr/local/admin/openadmin-amd64` (Go 1.27.3, SHA-256 `1d88ece9…742b1`)। REA দিয়ে build info আর string বিশ্লেষণ করা হয়েছে।
- `doc-guide/rust_recommended_stack_hosting_panel.pdf`
- `doc-guide/complete_license_anti_nulled_architecture.pdf`
