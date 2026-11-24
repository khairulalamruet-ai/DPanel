# Phase 02 — Node Agent, mTLS, Operation Ledger

**লক্ষ্য:** Control plane নিরাপদভাবে একটা server-এ typed কাজ পাঠাতে পারবে আর ফলাফল পাবে।
বিস্তারিত: [08-node-agent-protocol.md](../01-architecture/08-node-agent-protocol.md)

### T02.01 — Protocol সিদ্ধান্ত (ADR)
- **কী করতে হবে:** gRPC (tonic) নাকি WebSocket+JSON, তুলনা করে ADR লেখা। Version নীতি ঠিক করা।
- **DoD:** ADR merge হয়েছে।

### T02.02 — `node-protocol` crate
- **কী করতে হবে:** `OperationEnvelope`, `Operation` enum (শুধু `Ping`, `ReportCapabilities`), `OperationResult`, `AgentError`, version।
- **DoD:** serde round-trip test আছে।

### T02.03 — Internal CA
- **কী করতে হবে:** `rcgen` দিয়ে CA বানানো (`bpctl ca init`)। CA key `/etc/builtpanel/secrets`-এ (0600)। Cert issue করার function।
- **DoD:** CA দিয়ে সই করা cert `openssl verify` pass করে।

### T02.04 — nodes ও node_certificates migration
- **DoD:** টেবিল তৈরি হয়েছে, unique fingerprint সহ।

### T02.05 — Enrollment token
- **কী করতে হবে:** Admin API `POST /admin/nodes` → এক-বারের token (hash DB-তে), ১৫ মিনিট মেয়াদ। Install command তৈরি।
- **DoD:** Token দ্বিতীয়বার কাজ করে না।

### T02.06 — Enrollment endpoint
- **কী করতে হবে:** Agent CSR + token পাঠায় → যাচাই → cert দেওয়া হয় → `nodes` row তৈরি হয়।
- **DoD:** ভুল বা মেয়াদোত্তীর্ণ token দিলে 401 আসে। Audit লেখা হয়।

### T02.07 — Agent কঙ্কাল
- **কী করতে হবে:** `apps/node-agent`। Config, enroll কমান্ড, cert রাখা (0600), systemd unit, `builtpanel-agent` user।
- **DoD:** `node-agent enroll --token …` সফল হয়।

### T02.08 — Agent stream সংযোগ
- **কী করতে হবে:** Agent outbound mTLS-এ control plane-এ যুক্ত হয়। Reconnect + exponential backoff। Heartbeat।
- **DoD:** Control plane-এ node `online` দেখায়। Agent বন্ধ করলে ৬০ সেকেন্ডে `offline` হয়।

### T02.09 — Revocation
- **কী করতে হবে:** Handshake-এর সময় `revoked_at` যাচাই। Admin "revoke node" বোতাম।
- **DoD:** Revoke করা agent সংযোগ করতে পারে না।

### T02.10 — operations ledger
- **কী করতে হবে:** `operations` টেবিল। Service `enqueue(op)` (idempotency key সহ)। Status-এর পথ: `pending → dispatched → succeeded/failed/retryable`।
- **DoD:** একই idempotency key দিয়ে দুবার enqueue করলে একটাই row থাকে।

### T02.11 — Dispatcher (worker)
- **কী করতে হবে:** Worker pending op তুলে নেয় → node-এর stream-এ পাঠায় → timeout ও retry policy চালায় → ফলাফল লেখে → Redis pubsub event পাঠায়।
- **DoD:** `Ping` op end-to-end succeeded হয়।

### T02.12 — Agent-এর op handler কাঠামো
- **কী করতে হবে:** Op registry, envelope মেয়াদ যাচাই, idempotency cache (redb), structured error।
- **DoD:** একই op_id দুবার এলে আগের ফলাফলই ফেরত দেয়।

### T02.13 — ReportCapabilities
- **কী করতে হবে:** CPU, RAM, disk, kernel, Docker version, seccomp/AppArmor/userns আছে কিনা।
- **DoD:** Admin UI-তে node-এর তথ্য দেখা যায়।

### T02.14 — `docker-manager` crate
- **কী করতে হবে:** Bollard wrapper, শুধু অনুমোদিত কাজ: image pull (digest), container create (hardened default), start, stop, remove, inspect, stats। Generic exec নেই।
- **DoD:** Integration test (testcontainers বা local Docker) pass করে।

### T02.15 — priv-helper কঙ্কাল
- **কী করতে হবে:** Unix socket (0600, শুধু agent user), allowlist করা কাজের enum (শুরুতে `Noop`), নিজের validation।
- **DoD:** অজানা কাজ পাঠালে প্রত্যাখ্যান করে আর log লেখে।

### T02.16 — Node UI (admin)
- **কী করতে হবে:** Node তালিকা (অবস্থা, last seen, version, capacity), "Add node" wizard (install command copy করা), revoke।
- **DoD:** UI থেকে একটা নতুন node যোগ করা যায়।

### T02.17 — Operation UI
- **কী করতে হবে:** Admin-এর "Operations" পেজ (filter, retry বোতাম)। `useOperation` hook।
- **DoD:** ব্যর্থ op UI থেকে retry করা যায়।
