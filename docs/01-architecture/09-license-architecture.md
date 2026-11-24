# ০৯ — License ও Anti-Nulled Architecture

`doc-guide/complete_license_anti_nulled_architecture.pdf`-এর সারাংশ, Built Panel-এর জন্য সাজানো।

## দুটো আলাদা জিনিস
| | কোথায় চলে | কী করে |
|---|---|---|
| **License Authority (LA)** | আপনার (বিক্রেতার) central server | গ্রাহক, product, plan, license, activation, signing, release registry, integrity event |
| **License client + Security agent** | গ্রাহকের Built Panel installation-এ | entitlement যাচাই, heartbeat, daily integrity report |

LA আর Built Panel control plane আলাদা codebase-এ বা অন্তত আলাদা `apps/license-authority`-তে থাকবে, আর আলাদা server-এ deploy হবে।

## Cryptography
| বিষয় | পদ্ধতি |
|---|---|
| License document (entitlement) | Ed25519 signature, canonical JSON, `kid` সহ |
| Release manifest | Ed25519 (আলাদা release key) |
| Binary file | SHA-256 |
| Installation identity | Installation-প্রতি key pair, public key LA-তে register করা |
| Admin password | Argon2id |
| Private key | KMS/HSM। কখনো DB, source বা গ্রাহকের কাছে নয় |

## Lifecycle
1. LA-তে admin license তৈরি করেন।
2. Built Panel install-এর সময় `installation_id` (UUID) + key pair তৈরি হয়।
3. `POST /api/v1/licenses/activate`। Proof of possession (nonce সই করে)।
4. LA transaction-এ যাচাই করে (status, activation limit) → signed entitlement দেয় (`iss, aud, license_id, installation_id, features, iat, exp, jti, kid`)।
5. Built Panel public key দিয়ে যাচাই করে → `license_state`-এ রাখে → feature-gate-এ ব্যবহার করে।
6. Heartbeat হয় প্রতি ৬ ঘণ্টায়, entitlement refresh প্রতি ১২ ঘণ্টায়, offline grace ৭২ ঘণ্টা (সব configurable)।

## Daily integrity
- LA প্রতিদিন challenge দেয় (nonce, scan_id, expiry) → agent signed manifest যাচাই করে → critical binary-গুলোর SHA-256 মেলায় (control-api, worker, node-agent, license-client, priv-helper, updater, backup) → signed report পাঠায়।
- ফলাফল: `passed` / `failed` / `partial` / `stale` / `unverifiable`।

## Enforcement (গ্রাহকের ডেটা নিরাপদ রেখে)
| ঘটনা | প্রতিক্রিয়া |
|---|---|
| Invalid signature | entitlement বাতিল, নতুন করে validation |
| Binary mismatch (যাচাই করা) | **Restricted mode** |
| Agent সাড়া দেয় না | stale হিসেবে চিহ্নিত, notify, retry (tampering ধরা হবে না) |
| License revoked বা expired (grace-এর পরে) | Restricted mode |

**Restricted mode-এ বন্ধ থাকে:** panel management API, নতুন provisioning, config পরিবর্তন।
**চালু থাকে:** website, DB, mail, DNS, backup, log, recovery interface।
**কখনো করা যাবে না:** ডেটা মুছে ফেলা, এনক্রিপ্ট করা, website বন্ধ করা বা অন্য কোনো sabotage।

## Recovery
ঘটনা রেকর্ড করা হয় → tampering নাকি corruption বা ব্যর্থ update, সেটা আলাদা করা হয় → notify → official signed component দিয়ে reinstall বা rollback → আবার যাচাই → privilege ফেরত → audit।

## LA API (PDF থেকে)
Public: `activate`, `validate`, `refresh`, `deactivate`, `status`, `entitlements`, `heartbeat`, `integrity/report`, `releases/manifest`, `recovery/request`।
Admin: licenses CRUD, suspend, revoke, renew, activations, release, `keys/rotate`, `integrity/events`।

## LA DB (PDF থেকে)
`customers, products, plans, licenses, installations, activations, subscriptions, entitlements, signing_keys, release_manifests, release_components, integrity_scans, integrity_events, revocations, recovery_requests, admin_users, audit_logs, idempotency_keys`

## সীমাবদ্ধতা (সৎ থাকা)
গ্রাহকের নিয়ন্ত্রণে থাকা OS-এ যথেষ্ট privilege থাকলে আক্রমণকারী agent report জাল করতে পারে। এই architecture cracking কঠিন করে আর ধরা পড়ার সম্ভাবনা বাড়ায়, কিন্তু **অসম্ভব করে না**। Sensitive ফিচারের সিদ্ধান্ত server-side entitlement-এ রাখা সবচেয়ে কার্যকর।
