# Phase 17 — License Authority, License Client, Integrity

বিস্তারিত: [09-license-architecture.md](../01-architecture/09-license-architecture.md)। এটা PDF-এর ১১-phase roadmap-এর ছোট task-এ ভাগ করা রূপ।
**MVP-তে:** 17A + 17B। 17C (daily integrity) আর 17D (billing) পরে।

## 17A — License Authority (LA) ভিত্তি
### T17.01 — LA workspace
- **কী করতে হবে:** `apps/license-authority` (বা আলাদা repo) + `license-worker`। একই foundation (phase 00)।
- **DoD:** আলাদা DB আর config দিয়ে চলে।

### T17.02 — LA DB schema
- **কী করতে হবে:** PDF-এর ১৮টা টেবিল (customers … idempotency_keys)।
- **DoD:** Migration তৈরি হয়েছে।

### T17.03 — Signing service ও key
- **কী করতে হবে:** `crypto` crate-এ Ed25519। KMS adapter trait (প্রথমে file-based dev key, prod-এ KMS/HSM)। License key আর release key আলাদা। `kid`।
- **DoD:** Private key DB-তে নেই (test দিয়ে যাচাই)।

### T17.04 — Canonical serialization
- **কী করতে হবে:** Deterministic JSON (RFC 8785 JCS)। Schema version, iss, aud, kid, jti, iat, exp।
- **DoD:** একই payload সবসময় একই byte দেয় (property test)।

### T17.05 — Customer, product, plan, license CRUD (LA admin API)
- **DoD:** PDF-এর admin endpoint (`/admin/licenses`, suspend, revoke, renew) আছে।

### T17.06 — LA admin UI (React)
- **কী করতে হবে:** Overview, customer, license, activation, revoke (PDF-এর ১৩ নম্বর অংশ)।
- **DoD:** একটা license issue আর revoke করা যায়।

## 17B — Activation ও Entitlement
### T17.07 — Installation identity (Built Panel-এর দিকে)
- **কী করতে হবে:** Install-এর সময় `installation_id` + key pair। Private key 0600-এ রাখা।
- **DoD:** Reinstall করলেও identity থাকে (backup-এ যুক্ত)।

### T17.08 — Activate endpoint (LA)
- **কী করতে হবে:** Nonce challenge → proof of possession → transactional limit যাচাই → signed entitlement। Idempotent।
- **DoD:** Activation limit পার হলে প্রত্যাখ্যান করে।

### T17.09 — License client crate
- **কী করতে হবে:** Entitlement যাচাই (public key bundle, kid), claim যাচাই, `license_state` আপডেট।
- **DoD:** জাল বা পরিবর্তিত entitlement প্রত্যাখ্যাত হয়।

### T17.10 — Heartbeat ও refresh
- **কী করতে হবে:** Worker job: ৬ ঘণ্টা পর পর heartbeat, ১২ ঘণ্টা পর পর refresh, ৭২ ঘণ্টা offline grace (সব configurable)।
- **DoD:** Offline-এ ৭২ ঘণ্টা পর্যন্ত চলে, তারপর restricted mode।

### T17.11 — Entitlement → feature-gate
- **কী করতে হবে:** License-এর features আর limits (যেমন max tenant, max node) phase 03-এর feature-gate-এ যুক্ত করা।
- **DoD:** License-এর সীমার বেশি node যোগ করা যায় না।

### T17.12 — Restricted mode
- **কী করতে হবে:** Middleware: restricted হলে management API বন্ধ, read-only আর recovery চালু। **Website, DB, mail, DNS আর backup চলতে থাকবে।**
- **DoD:** Test: restricted অবস্থায় site 200 দেয়, আর নতুন domain যোগ করলে 423 আসে।

### T17.13 — License UI (Built Panel admin)
- **DoD:** License অবস্থা, মেয়াদ, feature, key প্রবেশ করানো, আবার যাচাই, recovery request।

## 17C — Release integrity (পরে)
### T17.14 — Release manifest pipeline (CI)
- **কী করতে হবে:** Build → প্রতিটা critical binary-র SHA-256 আর size → canonical manifest → release key দিয়ে সই → LA registry।
- **DoD:** CI artifact-এ signed manifest আছে।

### T17.15 — Update verifier
- **কী করতে হবে:** Download → signature যাচাই → hash যাচাই → atomic install → known-good rollback।
- **DoD:** একটা byte বদলালে update প্রত্যাখ্যাত হয়।

### T17.16 — Daily integrity scan
- **কী করতে হবে:** LA challenge → agent hash → signed report → LA মূল্যায়ন → event।
- **DoD:** Binary বদলালে পরের scan-এ `failed` আর event আসে।

### T17.17 — Integrity event ও recovery workflow
- **DoD:** Admin review → recovery → privilege ফেরত, সবকিছু audit হয়।

## 17D — Subscription ও billing (পরে)
### T17.18 — Payment webhook (signature যাচাই, idempotent)
### T17.19 — Subscription state machine (pending → active → past due → grace → suspended → cancelled/expired)
### T17.20 — Renewal ও notification
