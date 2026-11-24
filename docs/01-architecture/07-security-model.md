# ০৭ — নিরাপত্তা মডেল

## Threat model (সংক্ষেপে)
| হুমকি | কে | প্রতিরোধ |
|---|---|---|
| এক tenant অন্য tenant-এর ডেটায় ঢোকে | ক্ষতিকর গ্রাহক | আলাদা UID, user namespace, volume isolation, network policy, tenant_id scoping |
| Container escape | ক্ষতিকর গ্রাহক | non-root, cap_drop ALL, seccomp, AppArmor, no-new-privileges, read-only rootfs (যেখানে সম্ভব), উচ্চ ঝুঁকিতে gVisor |
| Panel-এ account takeover | বাইরের আক্রমণকারী | argon2id, 2FA, login rate limit, lockout, session revoke, নতুন IP-তে alert |
| CSRF, XSS | বাইরের | SameSite=Strict, CSRF token, কঠোর CSP, React-এর escaping, `dangerouslySetInnerHTML` নিষেধ |
| Command injection | input | shell নেই। শুধু typed op আর argv array। Path canonicalize + tenant root-এর ভেতরে কিনা যাচাই |
| Path traversal (file manager) | গ্রাহক | `openat2(RESOLVE_BENEATH)` বা সমতুল্য, symlink যাচাই |
| Control plane compromise | — | root নয়। Docker socket নেই। Agent শুধু typed op মানে, আর প্রতিটা op tenant ও node-এর সঙ্গে মেলায় |
| Agent impersonation | network | mTLS, node-প্রতি certificate, rotation, revocation |
| Resource abuse (DoS) | গ্রাহক | cgroup limit, rate limit, disk quota, inode quota, outbound mail limit |
| Supply chain | — | pinned dependency, cargo-deny/audit, image digest pinning, SBOM, image scan (Trivy) |

## Container hardening (প্রতিটা tenant container-এ বাধ্যতামূলক)
- `user: <tenant uid>:<gid>` (root নয়), `userns-remap` বা rootless Docker।
- `cap_drop: [ALL]`। কিছু যোগ করতে হলে নির্দিষ্টভাবে করতে হবে (যেমন web server-এর জন্য `NET_BIND_SERVICE` না দিয়ে উঁচু পোর্ট + Caddy)।
- `security_opt: no-new-privileges, seccomp=default, apparmor=builtpanel-tenant`।
- `pids_limit`, `memory`, `cpus`, `blkio` (plan অনুযায়ী)। Storage quota: XFS project quota বা ext4 quota।
- Mount: শুধু tenant-এর নিজের volume। Host bind mount নিষেধ, শুধু read-only template ছাড়া।
- Network: tenant-প্রতি bridge network। Inter-tenant traffic বন্ধ। Egress policy (SMTP 25 বন্ধ, শুধু mail relay দিয়ে পাঠানো যাবে)।

## Authentication ও session
- Password: argon2id (m=64MB, t=3, p=1, পরে tune করা যাবে)। Weak password list যাচাই হবে।
- Session: random 256-bit ID, Redis-এ রাখা। Idle timeout ও absolute timeout। Login-এর পর session ID rotate হবে।
- 2FA: TOTP (RFC 6238) + backup code। Admin-এর জন্য বাধ্যতামূলক (setting)।
- Sensitive action (delete tenant, root-level কাজ): "sudo mode", মানে password বা 2FA আবার যাচাই, ১০ মিনিটের জন্য।
- Admin-এর "login as user" (autologin): এক-বারের signed token, ৬০ সেকেন্ড মেয়াদ, audit-এ actor রেকর্ড হবে।

## Authorization
- RBAC: `role → permissions[]`। Permission key module manifest থেকে আসবে।
- Built-in role: `super_admin`, `admin`, `support` (শুধু পড়া + কিছু কাজ), `reseller`, `client`।
- প্রতিটা handler-এ `require(perm)` + tenant ownership যাচাই (client শুধু নিজের tenant দেখবে)।

## Audit
- প্রতিটা লেখার কাজ (create, update, delete, login, failed login, permission change) লিখতে হবে।
- Field: actor, actor_ip, action, target, tenant, node, outcome, correlation_id, metadata।
- DB role দিয়ে `UPDATE`/`DELETE` নিষেধ। Hash chain (প্রতিটা row-এ আগের row-এর hash) পরে যোগ করা হবে।

## Secret
- Tenant DB password, backup destination credential: `secret_enc` (XChaCha20-Poly1305, master key systemd credential বা KMS-এ)।
- Private signing key কখনো DB, image বা customer server-এ রাখা যাবে না (license PDF)।
