# ০৮ — Node Agent ও Protocol

## দায়িত্ব
- Control plane-এর **typed operation** গ্রহণ করে local Docker, Caddy, filesystem, quota আর firewall পরিবর্তন করা।
- Metric ও log stream পাঠানো।
- Reconcile loop চালানো: কাঙ্ক্ষিত অবস্থার সঙ্গে বাস্তব অবস্থা মেলানো।
- **কখনো** generic shell চালাবে না।

## সংযোগ
- **দিক:** Agent নিজেই control plane-এ সংযোগ করবে (outbound)। এতে node-এ inbound পোর্ট খোলা লাগে না, আর NAT-এর পেছনেও কাজ করে।
- **Transport:** HTTP/2 + mTLS, একটা দীর্ঘস্থায়ী bidirectional stream।
  - **প্রস্তাব:** `tonic` (gRPC) bidirectional stream। Typed, versioned, আর backpressure-সহ।
  - বিকল্প: WebSocket + JSON। সহজ, কিন্তু typing নিজেদের রাখতে হবে।
  - চূড়ান্ত সিদ্ধান্ত phase-02-এর T02.01-এ নেওয়া হবে।
- **Identity:** control plane-এর ভেতরে একটা ছোট CA থাকবে (key protected)। Enroll-এর সময় node CSR পাঠায় → এক-বার ব্যবহারযোগ্য enrollment token দিয়ে যাচাই হয় → ৯০ দিনের certificate দেওয়া হয় → মেয়াদ শেষের আগে নিজে থেকে rotate হয়।
- **Revocation:** `node_certificates.revoked_at`। Control plane handshake-এর সময় যাচাই করে।

## Enrollment-এর ধাপ
1. Admin "Add node" চাপেন → এক-বারের token তৈরি হয় (১৫ মিনিট মেয়াদ) আর install command দেখানো হয়।
2. Node-এ install script চলে → agent binary + `builtpanel-agent` user + systemd তৈরি হয়।
3. Agent key pair বানায় → CSR + token + host fingerprint পাঠায়।
4. Control plane যাচাই করে, cert দেয়, `nodes` row তৈরি করে।
5. Agent capability report পাঠায় (CPU, RAM, disk, kernel, Docker version, seccomp/AppArmor আছে কিনা)।

## Operation-এর রূপ (ধারণা)
```
OperationEnvelope {
  op_id: Uuid,            // operations.id
  idempotency_key: String,
  tenant_id: Option<Uuid>,
  issued_at, expires_at,  // replay রোধ
  protocol_version: u16,
  op: Operation,          // নিচের enum-এর একটা
}
OperationResult { op_id, status: Succeeded|Failed|Retryable, output: TypedOutput, error: Option<AgentError> }
```

## Operation তালিকা (MVP)
| Group | Operation |
|---|---|
| Node | `Ping`, `ReportCapabilities`, `CollectMetrics`, `Reconcile`, `SelfUpdate` |
| Tenant | `CreateTenant`, `ProvisionTenantStack`, `SetResourceLimits`, `SetStorageQuota`, `SuspendTenant`, `ResumeTenant`, `DeleteTenant` |
| Web | `AttachDomain`, `DetachDomain`, `SetDocroot`, `SetVhostPolicy`, `SetPhpVersion`, `ReloadWebServer` |
| SSL | `IssueCertificate`, `InstallCustomCertificate` |
| DB | `CreateDatabase`, `DropDatabase`, `UpsertDbUser`, `GrantDb`, `ImportSql` |
| Files | `ListDir`, `ReadFile`, `WriteFile`, `Move`, `Delete`, `Archive`, `Extract`, `FixPermissions`, `ScanPath` |
| FTP | `UpsertFtpAccount`, `DeleteFtpAccount` |
| Cron | `SyncCronJobs` |
| Backup | `CreateBackup`, `RestoreBackup`, `UploadBackup` |
| Security | `SetFirewallRules`, `SetWafPolicy` |
| Logs | `TailLogs` (stream), `ReadLog` |
| Container | `ListContainers`, `RestartContainer`, `PullImage` |

## প্রতিটা operation-এ agent যা যাচাই করে
1. Envelope-এর মেয়াদ, আর op_id আগে চলেছে কিনা (idempotency cache)।
2. `tenant_id` এই node-এর কিনা (local state DB, যেমন SQLite বা redb)।
3. Path tenant root-এর ভেতরে কিনা।
4. Image allowlist-এ আছে কিনা (digest)।
5. Resource limit plan-এর সর্বোচ্চ সীমার ভেতরে কিনা (control plane থেকে signed)।

## priv-helper
শুধু যে কাজে root লাগে: nftables rule প্রয়োগ, quota সেট, user/UID তৈরি, mount। Agent Unix socket দিয়ে নির্দিষ্ট JSON কাজ পাঠায়। Helper নিজে আবার validation করে। Binary ছোট রাখা হবে আর এর code review আলাদাভাবে হবে।
