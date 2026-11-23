import { http, HttpResponse } from "msw";

/** An error answer in the backend's problem-details shape (`code` is the stable machine code). */
export const problem = (status: number, code: string, detail: string) =>
  HttpResponse.json(
    { type: "about:blank", title: detail, status, detail, code },
    { status, headers: { "Content-Type": "application/problem+json" } },
  );

/**
 * MSW Mock Handlers
 * Real backend paths from repo-root openapi.json.
 */
let mfaEnabled = false;
const backupCodes = [
  "a1b2-c3d4",
  "e5f6-g7h8",
  "i9j0-k1l2",
  "m3n4-o5p6",
  "q7r8-s9t0",
  "u1v2-w3x4",
  "y5z6-a7b8",
  "c9d0-e1f2",
  "g3h4-i5j6",
  "k7l8-m9n0",
];

const getInitialSessions = () => [
  {
    id: "00000000-0000-0000-0000-000000000001",
    ip: "192.168.1.105",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/130.0",
    created_at: new Date(Date.now() - 3600 * 1000 * 2).toISOString(),
    last_seen_at: new Date().toISOString(),
    expires_at: new Date(Date.now() + 86400 * 1000 * 7).toISOString(),
    current: true,
  },
  {
    id: "00000000-0000-0000-0000-000000000002",
    ip: "203.0.113.42",
    user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Firefox/131.0",
    created_at: new Date(Date.now() - 86400 * 1000 * 3).toISOString(),
    last_seen_at: new Date(Date.now() - 3600 * 1000 * 5).toISOString(),
    expires_at: new Date(Date.now() + 86400 * 1000 * 4).toISOString(),
    current: false,
  },
  {
    id: "00000000-0000-0000-0000-000000000003",
    ip: "198.51.100.12",
    user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Mobile Safari/604.1",
    created_at: new Date(Date.now() - 86400 * 1000 * 7).toISOString(),
    last_seen_at: new Date(Date.now() - 86400 * 1000 * 2).toISOString(),
    expires_at: new Date(Date.now() + 86400 * 1000 * 1).toISOString(),
    current: false,
  },
];
let sessions = getInitialSessions();

const loginEvents = [
  {
    id: "evt_01",
    ip_address: "192.168.1.105",
    user_agent: "Chrome 130.0 (macOS)",
    status: "success" as const,
    created_at: new Date(Date.now() - 600 * 1000).toISOString(),
  },
  {
    id: "evt_02",
    ip_address: "192.168.1.105",
    user_agent: "Chrome 130.0 (macOS)",
    status: "2fa_prompt" as const,
    created_at: new Date(Date.now() - 650 * 1000).toISOString(),
  },
  {
    id: "evt_03",
    ip_address: "185.220.101.5",
    user_agent: "Python-urllib/3.10",
    status: "failed" as const,
    created_at: new Date(Date.now() - 3600 * 1000 * 4).toISOString(),
  },
  {
    id: "evt_04",
    ip_address: "203.0.113.42",
    user_agent: "Firefox 131.0 (Windows)",
    status: "success" as const,
    created_at: new Date(Date.now() - 86400 * 1000 * 3).toISOString(),
  },
];

export interface MockModuleItem {
  name: string;
  title: { en: string; bn: string };
  description: { en: string; bn: string };
  version: string;
  category: "core" | "web" | "db" | "files" | "mail" | "security" | "system";
  core: boolean;
  enabled: boolean;
  depends_on: string[];
  status: "active" | "disabled" | "error";
}

export const getInitialModules = (): MockModuleItem[] => [
  {
    name: "auth",
    title: { en: "Authentication", bn: "প্রমাণীকরণ" },
    description: {
      en: "User authentication, sessions, and multi-factor security",
      bn: "ব্যবহারকারী প্রমাণীকরণ, সেশন এবং টু-ফ্যাক্টর নিরাপত্তা",
    },
    version: "1.0.0",
    category: "core",
    core: true,
    enabled: true,
    depends_on: [],
    status: "active",
  },
  {
    name: "users",
    title: { en: "Users & RBAC", bn: "ব্যবহারকারী ও আরব্যাক" },
    description: {
      en: "User management and role-based access control",
      bn: "ব্যবহারকারী ব্যবস্থাপনা এবং ভূমিকা-ভিত্তিক অ্যাক্সেস নিয়ন্ত্রণ",
    },
    version: "1.0.0",
    category: "core",
    core: true,
    enabled: true,
    depends_on: ["auth"],
    status: "active",
  },
  {
    name: "settings",
    title: { en: "Settings", bn: "সেটিংস" },
    description: {
      en: "Global panel configuration and environment preferences",
      bn: "গ্লোবাল প্যানেল কনফিগারেশন এবং পরিবেশ সেটিংস",
    },
    version: "1.0.0",
    category: "core",
    core: true,
    enabled: true,
    depends_on: [],
    status: "active",
  },
  {
    name: "webserver",
    title: { en: "Web Server", bn: "ওয়েব সার্ভার" },
    description: {
      en: "Nginx / Traefik reverse proxy and vhost management",
      bn: "Nginx / Traefik রিভার্স প্রক্সি এবং ভার্চুয়াল হোস্ট পরিচালনা",
    },
    version: "1.0.0",
    category: "web",
    core: false,
    enabled: true,
    depends_on: [],
    status: "active",
  },
  {
    name: "dns",
    title: { en: "DNS Manager", bn: "ডিএনএস ম্যানেজার" },
    description: {
      en: "PowerDNS / CoreDNS zone files and record management",
      bn: "PowerDNS / CoreDNS জোন ফাইল এবং রেকর্ড ব্যবস্থাপনা",
    },
    version: "1.0.0",
    category: "web",
    core: false,
    enabled: true,
    depends_on: [],
    status: "active",
  },
  {
    name: "domains",
    title: { en: "Domains", bn: "ডোমেইন" },
    description: {
      en: "Domain routing, virtual hosts, and docroots",
      bn: "ডোমেইন রাউটিং, ভার্চুয়াল হোস্ট এবং ডকরুট",
    },
    version: "1.0.0",
    category: "web",
    core: false,
    enabled: true,
    depends_on: ["webserver", "dns"],
    status: "active",
  },
  {
    name: "db-mysql",
    title: { en: "MySQL / MariaDB", bn: "মাইএসকিউএল / মারিয়াডিবি" },
    description: {
      en: "Database container provisioning and user grants",
      bn: "ডাটাবেস কন্টেইনার প্রভিশনিং এবং ব্যবহারকারী গ্রান্ট",
    },
    version: "1.0.0",
    category: "db",
    core: false,
    enabled: true,
    depends_on: [],
    status: "active",
  },
  {
    name: "files",
    title: { en: "File Manager", bn: "ফাইল ম্যানেজার" },
    description: {
      en: "Web-based file explorer and permissions editor",
      bn: "ওয়েব-ভিত্তিক ফাইল ব্রাউজার এবং পারমিশন এডিটর",
    },
    version: "1.0.0",
    category: "files",
    core: false,
    enabled: false,
    depends_on: [],
    status: "disabled",
  },
];

let modules = getInitialModules();

export const resetMockModules = () => {
  modules = getInitialModules();
};

export const getInitialSettingsStore = () => ({
  general: {
    title: { en: "General", bn: "সাধারণ" },
    description: {
      en: "Global panel identity, default language, and session preferences",
      bn: "গ্লোবাল প্যানেল পরিচিতি, ডিফল্ট ভাষা এবং সেশন পছন্দসমূহ",
    },
    schema: {
      type: "object" as const,
      title: "General Settings",
      required: ["panel_name", "support_email"],
      properties: {
        panel_name: {
          type: "string",
          title: "Panel Name",
          description: "Name displayed across headers, emails, and login pages",
          default: "Built Panel",
        },
        support_email: {
          type: "string",
          format: "email",
          title: "Support Contact Email",
          description: "Inquiries and customer issue reports will be directed here",
          default: "support@builtpanel.local",
        },
        session_timeout: {
          type: "integer",
          title: "Session Timeout (Minutes)",
          description: "Duration of idle user session before automatic logout",
          default: 60,
          minimum: 5,
          maximum: 1440,
        },
        maintenance_mode: {
          type: "boolean",
          title: "Maintenance Mode",
          description: "Temporarily pause user access for platform maintenance",
          default: false,
        },
        default_locale: {
          type: "string",
          title: "Default Language",
          enum: ["bn", "en"],
          enumNames: ["বাংলা", "English"],
          default: "en",
        },
      },
    },
    values: {
      panel_name: "Built Panel",
      support_email: "support@builtpanel.local",
      session_timeout: 60,
      maintenance_mode: false,
      default_locale: "en",
    },
  },
  security: {
    title: { en: "Security", bn: "নিরাপত্তা" },
    description: {
      en: "Authentication policies, password rules, and brute-force protection",
      bn: "প্রমাণীকরণ নীতি, পাসওয়ার্ড নিয়ম এবং ব্রুট-ফোর্স সুরক্ষা",
    },
    schema: {
      type: "object" as const,
      title: "Security Settings",
      required: ["password_min_length", "max_login_attempts"],
      properties: {
        password_min_length: {
          type: "integer",
          title: "Minimum Password Length",
          description: "Minimum character count for new passwords",
          default: 8,
          minimum: 8,
          maximum: 64,
        },
        max_login_attempts: {
          type: "integer",
          title: "Max Failed Login Attempts",
          description: "Consecutive failed logins before temporary IP lockout",
          default: 5,
          minimum: 3,
          maximum: 20,
        },
        require_2fa_for_admins: {
          type: "boolean",
          title: "Enforce 2FA for Administrators",
          description: "Mandates TOTP setup for all admin accounts",
          default: true,
        },
        session_ip_binding: {
          type: "boolean",
          title: "Bind Sessions to IP Address",
          description: "Terminates session if request IP address changes",
          default: false,
        },
      },
    },
    values: {
      password_min_length: 8,
      max_login_attempts: 5,
      require_2fa_for_admins: true,
      session_ip_binding: false,
    },
  },
  backup: {
    title: { en: "Backups", bn: "ব্যাকআপ" },
    description: {
      en: "Automated snapshot frequency, compression, and retention policies",
      bn: "স্বয়ংক্রিয় স্ন্যাপশট ফ্রিকোয়েন্সি, কম্প্রেশন এবং রিটেনশন নীতি",
    },
    schema: {
      type: "object" as const,
      title: "Backup Settings",
      required: ["backup_retention_days", "compression_algorithm"],
      properties: {
        backup_retention_days: {
          type: "integer",
          title: "Retention Period (Days)",
          description: "Number of days before automated snapshots are pruned",
          default: 30,
          minimum: 1,
          maximum: 365,
        },
        compression_algorithm: {
          type: "string",
          title: "Compression Algorithm",
          enum: ["zstd", "gzip", "none"],
          enumNames: ["Zstandard (Fast & High Ratio)", "Gzip (Standard)", "None (Uncompressed)"],
          default: "zstd",
        },
        auto_backup_enabled: {
          type: "boolean",
          title: "Automatic Daily Backups",
          description: "Run automated backups every midnight",
          default: true,
        },
        remote_storage_provider: {
          type: "string",
          title: "Storage Destination",
          enum: ["local", "s3", "b2"],
          enumNames: ["Local Disk", "Amazon S3", "Backblaze B2"],
          default: "local",
        },
      },
    },
    values: {
      backup_retention_days: 30,
      compression_algorithm: "zstd",
      auto_backup_enabled: true,
      remote_storage_provider: "local",
    },
  },
  notifications: {
    title: { en: "Notifications", bn: "বিজ্ঞপ্তি" },
    description: {
      en: "Alert channels, email dispatches, and incoming webhooks",
      bn: "সতর্কবার্তা চ্যানেল, ইমেইল প্রেরণ এবং ইনকামিং ওয়েবহুক",
    },
    schema: {
      type: "object" as const,
      title: "Notification Settings",
      required: ["alert_email"],
      properties: {
        alert_email: {
          type: "string",
          format: "email",
          title: "Alert Recipient Email",
          description: "Destination address for critical node alarms",
          default: "alerts@builtpanel.local",
        },
        notify_on_node_offline: {
          type: "boolean",
          title: "Notify when a Node goes offline",
          description: "Dispatches immediate alert upon missed node heartbeat",
          default: true,
        },
        slack_webhook_url: {
          type: "string",
          title: "Slack Webhook URL",
          description: "Optional URL for posting critical system alerts to Slack channel",
          default: "",
        },
      },
    },
    values: {
      alert_email: "alerts@builtpanel.local",
      notify_on_node_offline: true,
      slack_webhook_url: "",
    },
  },
});

let mockSettingsStore: Record<string, any> = getInitialSettingsStore();

export const resetMockSettings = () => {
  mockSettingsStore = getInitialSettingsStore();
};

export interface MockFeatureSet {
  id: string;
  name: string;
  description: string;
  features: string[];
  is_default: boolean;
  plans_count: number;
  created_at: string;
  updated_at: string;
}

export const getInitialFeatureSets = (): MockFeatureSet[] => [
  {
    id: "fs_default",
    name: "Default (Full Stack)",
    description: "Standard full-featured web, database, email and DNS hosting",
    features: [
      "domains",
      "redirects",
      "docroot",
      "temporary_links",
      "ssl_custom",
      "ssl_letsencrypt",
      "dns_zones",
      "dns_records",
      "db_mysql",
      "db_remote",
      "phpmyadmin",
      "file_manager",
      "ftp_accounts",
      "email_inbox",
      "email_forwarding",
      "webmail",
      "ip_blocker",
      "modsecurity",
    ],
    is_default: true,
    plans_count: 3,
    created_at: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 2).toISOString(),
  },
  {
    id: "fs_basic",
    name: "Basic Web",
    description: "Essential web hosting with basic SSL and file manager",
    features: ["domains", "redirects", "docroot", "ssl_letsencrypt", "file_manager"],
    is_default: false,
    plans_count: 1,
    created_at: new Date(Date.now() - 86400 * 1000 * 10).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 5).toISOString(),
  },
  {
    id: "fs_dns_only",
    name: "DNS Only",
    description: "Dedicated nameserver and DNS zone record hosting",
    features: ["dns_zones", "dns_records"],
    is_default: false,
    plans_count: 0,
    created_at: new Date(Date.now() - 86400 * 1000 * 8).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 8).toISOString(),
  },
  {
    id: "fs_db_only",
    name: "Database Only",
    description: "Managed MySQL database instance with phpMyAdmin",
    features: ["db_mysql", "db_remote", "phpmyadmin"],
    is_default: false,
    plans_count: 0,
    created_at: new Date(Date.now() - 86400 * 1000 * 7).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 7).toISOString(),
  },
];

let featureSets = getInitialFeatureSets();

export const resetMockFeatureSets = () => {
  featureSets = getInitialFeatureSets();
};

export interface MockPlanResources {
  cpu: number;
  memory_mb: number;
  disk_mb: number;
  bandwidth_mb: number;
  pids_limit: number;
}

export interface MockPlanLimits {
  max_domains: number;
  max_subdomains: number;
  max_databases: number;
  max_ftp_accounts: number;
  max_email_accounts: number;
}

export interface MockPlan {
  id: string;
  name: string;
  description: string;
  feature_set_id: string;
  feature_set_name: string;
  is_default: boolean;
  users_count: number;
  resources: MockPlanResources;
  limits: MockPlanLimits;
  created_at: string;
  updated_at: string;
  node_group_id?: string | null;
  node_group_name?: string | null;
  target_tags?: string[];
  target_region?: string | null;
}

export const getInitialPlans = (): MockPlan[] => [
  {
    id: "plan_starter",
    name: "Starter Cloud",
    description: "Entry-level tier for personal blogs and landing pages",
    feature_set_id: "fs_basic",
    feature_set_name: "Basic Web",
    is_default: false,
    users_count: 8,
    node_group_id: "ng_global",
    node_group_name: "Global Cluster (Default)",
    target_region: "global",
    target_tags: ["general", "standard"],
    resources: {
      cpu: 1,
      memory_mb: 1024,
      disk_mb: 10240,
      bandwidth_mb: 51200,
      pids_limit: 100,
    },
    limits: {
      max_domains: 1,
      max_subdomains: 5,
      max_databases: 1,
      max_ftp_accounts: 2,
      max_email_accounts: 5,
    },
    created_at: new Date(Date.now() - 86400 * 1000 * 20).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 5).toISOString(),
  },
  {
    id: "plan_standard",
    name: "Standard Business",
    description: "Balanced hosting plan for business sites and eCommerce stores",
    feature_set_id: "fs_default",
    feature_set_name: "Default (Full Stack)",
    is_default: true,
    users_count: 24,
    node_group_id: "ng_us_east",
    node_group_name: "US-East General Pool",
    target_region: "us-east-1",
    target_tags: ["us-east", "nvme", "ssd"],
    resources: {
      cpu: 2,
      memory_mb: 2048,
      disk_mb: 25600,
      bandwidth_mb: -1,
      pids_limit: 256,
    },
    limits: {
      max_domains: 5,
      max_subdomains: 20,
      max_databases: 5,
      max_ftp_accounts: 5,
      max_email_accounts: 20,
    },
    created_at: new Date(Date.now() - 86400 * 1000 * 15).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 2).toISOString(),
  },
  {
    id: "plan_pro",
    name: "Pro Developer",
    description: "High-performance compute and database allocation for web agencies",
    feature_set_id: "fs_default",
    feature_set_name: "Default (Full Stack)",
    is_default: false,
    users_count: 5,
    node_group_id: "ng_sgp_highmem",
    node_group_name: "Singapore High-RAM Pool",
    target_region: "ap-southeast-1",
    target_tags: ["apac", "highmem", "high-cpu"],
    resources: {
      cpu: 4,
      memory_mb: 4096,
      disk_mb: 51200,
      bandwidth_mb: -1,
      pids_limit: 512,
    },
    limits: {
      max_domains: 20,
      max_subdomains: 50,
      max_databases: 20,
      max_ftp_accounts: 20,
      max_email_accounts: -1,
    },
    created_at: new Date(Date.now() - 86400 * 1000 * 10).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 1).toISOString(),
  },
  {
    id: "plan_enterprise",
    name: "Enterprise Dedicated",
    description: "Uncapped resources and unlimited domains for enterprise clients",
    feature_set_id: "fs_default",
    feature_set_name: "Default (Full Stack)",
    is_default: false,
    users_count: 2,
    node_group_id: "ng_fra_isolated",
    node_group_name: "Frankfurt EU-West Group",
    target_region: "eu-central-1",
    target_tags: ["eu", "gdpr", "dedicated"],
    resources: {
      cpu: -1,
      memory_mb: -1,
      disk_mb: -1,
      bandwidth_mb: -1,
      pids_limit: -1,
    },
    limits: {
      max_domains: -1,
      max_subdomains: -1,
      max_databases: -1,
      max_ftp_accounts: -1,
      max_email_accounts: -1,
    },
    created_at: new Date(Date.now() - 86400 * 1000 * 5).toISOString(),
    updated_at: new Date(Date.now() - 86400 * 1000 * 5).toISOString(),
  },
];

let plans = getInitialPlans();

export const resetMockPlans = () => {
  plans = getInitialPlans();
};

export interface MockUser {
  id: string;
  username: string;
  email: string;
  status: "active" | "suspended" | "pending";
  plan_id: string;
  plan_name: string;
  node_id: string;
  node_name: string;
  node_status?: "online" | "offline" | "draining" | "revoked";
  is_node_offline?: boolean;
  primary_domain: string;
  disk_used_bytes: number;
  disk_limit_bytes: number;
  bandwidth_used_bytes: number;
  bandwidth_limit_bytes: number;
  domains_count: number;
  created_at: string;
  suspended_at: string | null;
  suspended_reason: string | null;
}

const mockPlansList = [
  { id: "plan_starter", name: "Starter Cloud", disk_limit: 10 * 1024 * 1024 * 1024 },
  { id: "plan_standard", name: "Standard Business", disk_limit: 25 * 1024 * 1024 * 1024 },
  { id: "plan_pro", name: "Pro Developer", disk_limit: 50 * 1024 * 1024 * 1024 },
  { id: "plan_enterprise", name: "Enterprise Dedicated", disk_limit: -1 },
];

const mockNodesList = [
  { id: "node-sgp-01", name: "node-sgp-01 (Singapore)", status: "online" as const },
  { id: "node-fra-01", name: "node-fra-01 (Frankfurt)", status: "online" as const },
  { id: "node-nyc-01", name: "node-nyc-01 (New York)", status: "offline" as const },
];

export const getInitialUsers = (): MockUser[] => {
  const usersList: MockUser[] = [];
  const baseUsernames = [
    "alpha", "beta", "gamma", "delta", "nexus", "vertex", "horizon", "aurora",
    "zenith", "stellar", "quantum", "solaris", "vortex", "cosmic", "prism",
    "cyber", "apex", "ember", "titan", "phoenix",
  ];

  for (let i = 1; i <= 1000; i++) {
    const base = baseUsernames[(i - 1) % baseUsernames.length];
    const username = i <= 20 ? base : `${base}_${i}`;
    const plan = mockPlansList[(i - 1) % mockPlansList.length];
    const node = mockNodesList[(i - 1) % mockNodesList.length];

    const status: "active" | "suspended" | "pending" =
      i % 10 === 0 ? "suspended" : i % 20 === 0 ? "pending" : "active";

    const diskLimit = plan.disk_limit;
    const diskUsed =
      diskLimit === -1
        ? 4294967296
        : Math.min(diskLimit, ((i * 12345678) % (10 * 1024 * 1024 * 1024)) + 104857600);

    usersList.push({
      id: `usr_${String(i).padStart(4, "0")}`,
      username,
      email: `${username}@example.com`,
      status,
      plan_id: plan.id,
      plan_name: plan.name,
      node_id: node.id,
      node_name: node.name,
      node_status: node.status,
      is_node_offline: node.status === "offline",
      primary_domain: `${username}.com`,
      disk_used_bytes: diskUsed,
      disk_limit_bytes: diskLimit,
      bandwidth_used_bytes: (i * 9876543) % (50 * 1024 * 1024 * 1024),
      bandwidth_limit_bytes: -1,
      domains_count: (i % 5) + 1,
      created_at: new Date(Date.now() - i * 86400 * 1000).toISOString(),
      suspended_at:
        status === "suspended" ? new Date(Date.now() - 3600 * 1000 * 12).toISOString() : null,
      suspended_reason: status === "suspended" ? "Terms of service violation review" : null,
    });
  }
  return usersList;
};

let mockUsers = getInitialUsers();

export const resetMockUsers = () => {
  mockUsers = getInitialUsers();
  mockDetailStore = {};
};

export let mockDetailStore: Record<string, any> = {};

interface MockClientDomain {
  id: string;
  name: string;
  type: "primary" | "addon" | "subdomain" | "alias";
  docroot: string;
  is_primary: boolean;
  ssl_status: "active" | "pending" | "expired" | "self_signed" | "none";
  ssl_expires_at?: string | null;
  ssl_issuer?: string | null;
  dns_status: "active" | "pending" | "proxied" | "error";
  redirect?: {
    enabled: boolean;
    target_url: string;
    status_code: 301 | 302;
  } | null;
  php_version?: string;
  created_at: string;
}

const getInitialClientDomains = (): MockClientDomain[] => [
  {
    id: "dom_01",
    name: "alpha-hosting.com",
    type: "primary",
    docroot: "/domains/alpha-hosting.com/public_html",
    is_primary: true,
    ssl_status: "active",
    ssl_expires_at: new Date(Date.now() + 86400 * 1000 * 76).toISOString(),
    ssl_issuer: "Let's Encrypt Authority X3",
    dns_status: "active",
    redirect: null,
    php_version: "PHP 8.3",
    created_at: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
  },
  {
    id: "dom_02",
    name: "blog.alpha-hosting.com",
    type: "subdomain",
    docroot: "/domains/blog.alpha-hosting.com/public_html",
    is_primary: false,
    ssl_status: "active",
    ssl_expires_at: new Date(Date.now() + 86400 * 1000 * 60).toISOString(),
    ssl_issuer: "Let's Encrypt Authority X3",
    dns_status: "active",
    redirect: null,
    php_version: "PHP 8.3",
    created_at: new Date(Date.now() - 86400 * 1000 * 10).toISOString(),
  },
  {
    id: "dom_03",
    name: "shop.alpha-hosting.com",
    type: "subdomain",
    docroot: "/domains/shop.alpha-hosting.com/public_html",
    is_primary: false,
    ssl_status: "pending",
    dns_status: "pending",
    redirect: null,
    php_version: "PHP 8.3",
    created_at: new Date(Date.now() - 3600 * 1000 * 2).toISOString(),
  },
];

let mockClientDomains = getInitialClientDomains();

export const resetMockClientDomains = () => {
  mockClientDomains = getInitialClientDomains();
};

/** A client domain as the backend's `DomainView` has it. */
const clientDomainView = (d: MockClientDomain) => {
  const cert = mockSslCertificates.find((c) => c.domain_id === d.id);
  return {
    id: d.id,
    fqdn: d.name,
    unicode: d.name,
    kind: d.type === "subdomain" ? "sub" : d.type,
    docroot: d.docroot.replace(/^\/+/, ""),
    force_https: cert?.force_https ?? false,
    hsts: cert?.hsts_enabled ?? false,
    status: "active",
    parent_domain_id: null,
    tenant_id: "usr_0001",
    tenant_username: "demo",
    suspended_at: null,
    created_at: d.created_at,
  };
};

/** The certificate state of a domain as the backend's `SslView` has it. */
const sslViewOf = (domainId: string) => {
  const domain = mockClientDomains.find((d) => d.id === domainId);
  const cert = mockSslCertificates.find((c) => c.domain_id === domainId);
  if (!domain) return null;
  const status = cert?.status ?? domain.ssl_status;
  const days = cert?.days_remaining ?? 0;
  const state = status === "active" ? (days <= 14 ? "expiring" : "valid") : status === "expired" ? "expired" : "none";
  return {
    state,
    kind: cert?.type === "custom" ? "custom" : state === "none" ? null : "acme",
    issuer: cert?.issuer ?? domain.ssl_issuer ?? null,
    not_before: cert?.created_at ?? null,
    not_after: cert?.expires_at ?? domain.ssl_expires_at ?? null,
    days_left: state === "none" ? null : days,
    last_seen_at: cert?.updated_at ?? null,
  };
};

interface MockAdminDomain {
  id: string;
  name: string;
  type: "primary" | "addon" | "subdomain" | "alias";
  tenant_id: string;
  tenant_username: string;
  node_id: string;
  node_name: string;
  docroot: string;
  is_primary: boolean;
  status: "active" | "suspended";
  suspended_at?: string | null;
  suspended_reason?: string | null;
  ssl_status: "active" | "pending" | "expired" | "self_signed" | "none";
  ssl_expires_at?: string | null;
  ssl_issuer?: string | null;
  dns_status: "active" | "pending" | "proxied" | "error";
  redirect?: {
    enabled: boolean;
    target_url: string;
    status_code: 301 | 302;
  } | null;
  php_version?: string;
  created_at: string;
}

const getInitialAdminDomains = (): MockAdminDomain[] => {
  const tenants = [
    { id: "usr_0001", username: "alpha", node: "node-sgp-01", node_name: "node-sgp-01 (Singapore)" },
    { id: "usr_0002", username: "beta", node: "node-master-01", node_name: "node-master-01 (US-East)" },
    { id: "usr_0003", username: "gamma", node: "node-worker-02", node_name: "node-worker-02 (EU-Frankfurt)" },
    { id: "usr_0004", username: "delta", node: "node-sgp-01", node_name: "node-sgp-01 (Singapore)" },
  ];

  const items: MockAdminDomain[] = [];
  tenants.forEach((t, idx) => {
    items.push({
      id: `adm_dom_${idx + 1}`,
      name: `${t.username}-hosting.com`,
      type: "primary",
      tenant_id: t.id,
      tenant_username: t.username,
      node_id: t.node,
      node_name: t.node_name,
      docroot: `/domains/${t.username}-hosting.com/public_html`,
      is_primary: true,
      status: "active",
      ssl_status: "active",
      ssl_expires_at: new Date(Date.now() + 86400 * 1000 * 75).toISOString(),
      ssl_issuer: "Let's Encrypt Authority X3",
      dns_status: "active",
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date(Date.now() - 86400 * 1000 * (15 - idx)).toISOString(),
    });
  });

  items.push(
    {
      id: "adm_dom_05",
      name: "shop.alpha-hosting.com",
      type: "subdomain",
      tenant_id: "usr_0001",
      tenant_username: "alpha",
      node_id: "node-sgp-01",
      node_name: "node-sgp-01 (Singapore)",
      docroot: "/domains/shop.alpha-hosting.com/public_html",
      is_primary: false,
      status: "active",
      ssl_status: "active",
      ssl_expires_at: new Date(Date.now() + 86400 * 1000 * 60).toISOString(),
      ssl_issuer: "Let's Encrypt Authority X3",
      dns_status: "active",
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date(Date.now() - 86400 * 1000 * 10).toISOString(),
    },
    {
      id: "adm_dom_06",
      name: "blog.beta-hosting.com",
      type: "subdomain",
      tenant_id: "usr_0002",
      tenant_username: "beta",
      node_id: "node-master-01",
      node_name: "node-master-01 (US-East)",
      docroot: "/domains/blog.beta-hosting.com/public_html",
      is_primary: false,
      status: "suspended",
      ssl_status: "expired",
      dns_status: "active",
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date(Date.now() - 86400 * 1000 * 8).toISOString(),
    },
    {
      id: "adm_dom_07",
      name: "landing.gamma-hosting.com",
      type: "addon",
      tenant_id: "usr_0003",
      tenant_username: "gamma",
      node_id: "node-worker-02",
      node_name: "node-worker-02 (EU-Frankfurt)",
      docroot: "/domains/landing.gamma-hosting.com/public_html",
      is_primary: false,
      status: "active",
      ssl_status: "pending",
      dns_status: "pending",
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date(Date.now() - 86400 * 1000 * 3).toISOString(),
    }
  );

  return items;
};

let mockAdminDomains = getInitialAdminDomains();

export const resetMockAdminDomains = () => {
  mockAdminDomains = getInitialAdminDomains();
};

const getInitialOperations = (): any[] => [
  {
    id: "00000000-0000-0000-0000-000000000101",
    idempotency_key: "idemp_ping_001",
    op_type: "Ping",
    node_id: "00000000-0000-0000-0000-0000000000aa",
    node_name: "node-master-01",
    tenant_id: null,
    status: "succeeded",
    retries_count: 0,
    payload: { echo: "builtpanel-heartbeat", timestamp: new Date(Date.now() - 60000).toISOString() },
    output: { pong: true, latency_ms: 1.4 },
    error: null,
    created_at: new Date(Date.now() - 65000).toISOString(),
    updated_at: new Date(Date.now() - 60000).toISOString(),
  },
  {
    id: "00000000-0000-0000-0000-000000000102",
    idempotency_key: "idemp_domain_002",
    op_type: "AttachDomain",
    node_id: "00000000-0000-0000-0000-0000000000aa",
    node_name: "node-master-01",
    tenant_id: "00000000-0000-0000-0000-000000000001",
    status: "succeeded",
    retries_count: 0,
    payload: { domain: "example.com", docroot: "/var/www/example.com", php: "8.3" },
    output: { vhost_created: true, caddy_reloaded: true },
    error: null,
    created_at: new Date(Date.now() - 120000).toISOString(),
    updated_at: new Date(Date.now() - 110000).toISOString(),
  },
  {
    id: "00000000-0000-0000-0000-000000000103",
    idempotency_key: "idemp_pull_003",
    op_type: "PullImage",
    node_id: "00000000-0000-0000-0000-0000000000bb",
    node_name: "node-worker-02",
    tenant_id: null,
    status: "failed",
    retries_count: 2,
    payload: { image: "redis:7.4-alpine", digest: "sha256:7f9202a061b" },
    output: null,
    error: {
      code: "image_pull_timeout",
      message: "Network timeout while pulling container image layer 3 from registry",
      details: { registry: "docker.io", layer: "sha256:4a8bc9...", elapsed_seconds: 45 },
    },
    created_at: new Date(Date.now() - 300000).toISOString(),
    updated_at: new Date(Date.now() - 250000).toISOString(),
  },
  {
    id: "00000000-0000-0000-0000-000000000104",
    idempotency_key: "idemp_db_004",
    op_type: "CreateDatabase",
    node_id: "00000000-0000-0000-0000-0000000000bb",
    node_name: "node-worker-02",
    tenant_id: "00000000-0000-0000-0000-000000000001",
    status: "retryable",
    retries_count: 1,
    payload: { name: "demo_db", user: "db_user", charset: "utf8mb4" },
    output: null,
    error: {
      code: "connection_refused",
      message: "Database socket connection refused: daemon is initializing",
      details: { socket: "/var/run/mysqld/mysqld.sock" },
    },
    created_at: new Date(Date.now() - 400000).toISOString(),
    updated_at: new Date(Date.now() - 350000).toISOString(),
  },
  {
    id: "00000000-0000-0000-0000-000000000105",
    idempotency_key: "idemp_reload_005",
    op_type: "ReloadWebServer",
    node_id: "00000000-0000-0000-0000-0000000000aa",
    node_name: "node-master-01",
    tenant_id: null,
    status: "dispatched",
    retries_count: 0,
    payload: { graceful: true },
    output: null,
    error: null,
    created_at: new Date(Date.now() - 5000).toISOString(),
    updated_at: new Date(Date.now() - 3000).toISOString(),
  },
];

let mockOperations = getInitialOperations();

export const resetMockOperations = () => {
  mockOperations = getInitialOperations();
};

interface MockDnsRecord {
  id: string;
  zone_id: string;
  name: string;
  type: string;
  content: string;
  ttl: number;
  priority?: number | null;
  weight?: number | null;
  port?: number | null;
  flags?: number | null;
  tag?: string | null;
  created_at: string;
  updated_at: string;
}

interface MockDnsZone {
  id: string;
  domain_id: string;
  domain_name: string;
  tenant_id: string;
  nameservers: string[];
  soa_serial: number;
  records_count: number;
  is_custom_ns: boolean;
  dnssec_enabled: boolean;
  updated_at: string;
}

const getInitialDnsZones = (): MockDnsZone[] => [
  {
    id: "zone_alpha_01",
    domain_id: "dom_01",
    domain_name: "alpha-hosting.com",
    tenant_id: "usr_0001",
    nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
    soa_serial: 2026111301,
    records_count: 7,
    is_custom_ns: false,
    dnssec_enabled: true,
    updated_at: new Date(Date.now() - 3600 * 1000).toISOString(),
  },
  {
    id: "zone_alpha_02",
    domain_id: "dom_02",
    domain_name: "blog.alpha-hosting.com",
    tenant_id: "usr_0001",
    nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
    soa_serial: 2026111302,
    records_count: 2,
    is_custom_ns: false,
    dnssec_enabled: false,
    updated_at: new Date(Date.now() - 7200 * 1000).toISOString(),
  },
  {
    id: "zone_beta_01",
    domain_id: "dom_03",
    domain_name: "shop.alpha-hosting.com",
    tenant_id: "usr_0001",
    nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
    soa_serial: 2026111303,
    records_count: 2,
    is_custom_ns: false,
    dnssec_enabled: false,
    updated_at: new Date(Date.now() - 86400 * 1000).toISOString(),
  },
];

const getInitialDnsRecords = (): MockDnsRecord[] => [
  {
    id: "rec_01",
    zone_id: "zone_alpha_01",
    name: "@",
    type: "A",
    content: "192.168.1.100",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_02",
    zone_id: "zone_alpha_01",
    name: "www",
    type: "CNAME",
    content: "alpha-hosting.com.",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_03",
    zone_id: "zone_alpha_01",
    name: "@",
    type: "MX",
    content: "mail.alpha-hosting.com.",
    ttl: 3600,
    priority: 10,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_04",
    zone_id: "zone_alpha_01",
    name: "@",
    type: "TXT",
    content: "v=spf1 a mx ~all",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_05",
    zone_id: "zone_alpha_01",
    name: "@",
    type: "AAAA",
    content: "2001:db8::1",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_06",
    zone_id: "zone_alpha_01",
    name: "mail",
    type: "A",
    content: "192.168.1.105",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_07",
    zone_id: "zone_alpha_01",
    name: "@",
    type: "CAA",
    content: "0 issue \"letsencrypt.org\"",
    ttl: 3600,
    flags: 0,
    tag: "issue",
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_08",
    zone_id: "zone_alpha_02",
    name: "@",
    type: "A",
    content: "192.168.1.102",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_09",
    zone_id: "zone_alpha_02",
    name: "www",
    type: "CNAME",
    content: "blog.alpha-hosting.com.",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_10",
    zone_id: "zone_beta_01",
    name: "@",
    type: "A",
    content: "192.168.1.110",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
  {
    id: "rec_11",
    zone_id: "zone_beta_01",
    name: "www",
    type: "CNAME",
    content: "beta-app.io.",
    ttl: 3600,
    priority: null,
    created_at: new Date(Date.now() - 86400000).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
  },
];

let mockDnsZones = getInitialDnsZones();
let mockDnsRecords = getInitialDnsRecords();

export interface MockDnsClusterServer {
  id: string;
  name: string;
  hostname: string;
  ip_address: string;
  port: number;
  api_url: string;
  role: "primary" | "secondary" | "replica";
  status: "online" | "offline" | "syncing" | "error";
  sync_strategy: "native_db" | "axfr" | "api_push";
  zones_count: number;
  last_sync_at: string;
  latency_ms: number;
  version?: string;
  is_draining?: boolean;
}

export const getInitialDnsClusterServers = (): MockDnsClusterServer[] => [
  {
    id: "dns_srv_01",
    name: "Primary PowerDNS US-East",
    hostname: "ns1.builtpanel.internal",
    ip_address: "192.0.2.11",
    port: 53,
    api_url: "http://192.0.2.11:8081",
    role: "primary",
    status: "online",
    sync_strategy: "native_db",
    zones_count: 14,
    last_sync_at: new Date(Date.now() - 60000).toISOString(),
    latency_ms: 12,
    version: "PowerDNS Authoritative 4.9.1",
  },
  {
    id: "dns_srv_02",
    name: "Secondary PowerDNS EU-Central",
    hostname: "ns2.builtpanel.internal",
    ip_address: "198.51.100.22",
    port: 53,
    api_url: "http://198.51.100.22:8081",
    role: "secondary",
    status: "online",
    sync_strategy: "axfr",
    zones_count: 14,
    last_sync_at: new Date(Date.now() - 120000).toISOString(),
    latency_ms: 45,
    version: "PowerDNS Authoritative 4.9.1",
  },
  {
    id: "dns_srv_03",
    name: "Edge Replica PowerDNS AP-South",
    hostname: "ns3.builtpanel.internal",
    ip_address: "203.0.113.33",
    port: 53,
    api_url: "http://203.0.113.33:8081",
    role: "replica",
    status: "online",
    sync_strategy: "api_push",
    zones_count: 14,
    last_sync_at: new Date(Date.now() - 180000).toISOString(),
    latency_ms: 88,
    version: "PowerDNS Authoritative 4.9.1",
  },
];

let mockDnsClusterServers = getInitialDnsClusterServers();

export const resetMockDns = () => {
  mockDnsZones = getInitialDnsZones();
  mockDnsRecords = getInitialDnsRecords();
  mockDnsClusterServers = getInitialDnsClusterServers();
};

interface MockSslCert {
  id: string;
  domain_id: string;
  domain_name: string;
  status: "active" | "pending" | "expired" | "self_signed" | "none";
  type: "lets_encrypt" | "zerossl" | "custom" | "self_signed" | "none";
  issuer: string | null;
  subject: string;
  sans: string[];
  expires_at: string | null;
  days_remaining: number;
  auto_renew: boolean;
  force_https: boolean;
  hsts_enabled: boolean;
  hsts_subdomains: boolean;
  hsts_preload: boolean;
  created_at: string;
  updated_at: string;
  cert_fingerprint?: string | null;
}

const getInitialSslCertificates = (): MockSslCert[] => [
  {
    id: "cert_01",
    domain_id: "dom_01",
    domain_name: "alpha-hosting.com",
    status: "active",
    type: "lets_encrypt",
    issuer: "Let's Encrypt Authority X3",
    subject: "CN=alpha-hosting.com",
    sans: ["alpha-hosting.com", "www.alpha-hosting.com"],
    expires_at: new Date(Date.now() + 76 * 86400 * 1000).toISOString(),
    days_remaining: 76,
    auto_renew: true,
    force_https: true,
    hsts_enabled: true,
    hsts_subdomains: true,
    hsts_preload: false,
    created_at: new Date(Date.now() - 14 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 14 * 86400 * 1000).toISOString(),
    cert_fingerprint: "SHA256:4a:8b:c9:e1:2f:33:9a:b4:c7:d2:e5:6f:88:99:aa:bb",
  },
  {
    id: "cert_02",
    domain_id: "dom_02",
    domain_name: "blog.alpha-hosting.com",
    status: "active",
    type: "custom",
    issuer: "DigiCert Global CA",
    subject: "CN=blog.alpha-hosting.com",
    sans: ["blog.alpha-hosting.com"],
    expires_at: new Date(Date.now() + 12 * 86400 * 1000).toISOString(),
    days_remaining: 12,
    auto_renew: true,
    force_https: false,
    hsts_enabled: false,
    hsts_subdomains: false,
    hsts_preload: false,
    created_at: new Date(Date.now() - 78 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 78 * 86400 * 1000).toISOString(),
    cert_fingerprint: "SHA256:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00",
  },
  {
    id: "cert_03",
    domain_id: "dom_03",
    domain_name: "shop.alpha-hosting.com",
    status: "expired",
    type: "zerossl",
    issuer: "ZeroSSL RSA Domain Secure Site CA",
    subject: "CN=beta-app.io",
    sans: ["beta-app.io", "*.beta-app.io"],
    expires_at: new Date(Date.now() - 2 * 86400 * 1000).toISOString(),
    days_remaining: 0,
    auto_renew: false,
    force_https: false,
    hsts_enabled: false,
    hsts_subdomains: false,
    hsts_preload: false,
    created_at: new Date(Date.now() - 92 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 2 * 86400 * 1000).toISOString(),
    cert_fingerprint: "SHA256:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99",
  },
];

let mockSslCertificates = getInitialSslCertificates();

export const resetMockSsl = () => {
  mockSslCertificates = getInitialSslCertificates();
};

export interface MockVhostTemplate {
  id: string;
  name: string;
  slug: string;
  server_type: "nginx" | "caddy";
  content: string;
  variables: string[];
  version: number;
  is_default: boolean;
  domain_count: number;
  created_at: string;
  updated_at: string;
}

export interface MockVhostVersion {
  id: string;
  template_id: string;
  version: number;
  content: string;
  changelog: string;
  created_by: string;
  created_at: string;
}

const getInitialVhostTemplates = (): MockVhostTemplate[] => [
  {
    id: "tpl_01",
    name: "Nginx PHP-FPM Standard",
    slug: "nginx-php-fpm",
    server_type: "nginx",
    content: `server {
    listen 80;
    listen [::]:80;
    server_name {{domain}} www.{{domain}};
    root {{docroot}};
    index index.php index.html index.htm;

    access_log /var/log/nginx/{{domain}}.access.log;
    error_log /var/log/nginx/{{domain}}.error.log;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \\.php$ {
        fastcgi_pass unix:{{php_socket}};
        fastcgi_index index.php;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        include fastcgi_params;
    }

    location ~ /\\.(?!well-known).* {
        deny all;
    }
}`,
    variables: ["domain", "docroot", "php_socket"],
    version: 3,
    is_default: true,
    domain_count: 3,
    created_at: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 2 * 86400 * 1000).toISOString(),
  },
  {
    id: "tpl_02",
    name: "Nginx Static Site",
    slug: "nginx-static",
    server_type: "nginx",
    content: `server {
    listen 80;
    listen [::]:80;
    server_name {{domain}} www.{{domain}};
    root {{docroot}};
    index index.html index.htm;

    access_log /var/log/nginx/{{domain}}.access.log;
    error_log /var/log/nginx/{{domain}}.error.log;

    location / {
        try_files $uri $uri/ =404;
    }

    location ~* \\.(jpg|jpeg|png|gif|ico|css|js)$ {
        expires 30d;
        add_header Cache-Control "public, no-transform";
    }
}`,
    variables: ["domain", "docroot"],
    version: 1,
    is_default: false,
    domain_count: 1,
    created_at: new Date(Date.now() - 40 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 40 * 86400 * 1000).toISOString(),
  },
  {
    id: "tpl_03",
    name: "Caddy Reverse Proxy",
    slug: "caddy-reverse-proxy",
    server_type: "caddy",
    content: `{{domain}}, www.{{domain}} {
    encode gzip zstd
    log {
        output file /var/log/caddy/{{domain}}.log
    }

    reverse_proxy 127.0.0.1:{{port}} {
        header_up Host {host}
        header_up X-Real-IP {remote}
    }
}`,
    variables: ["domain", "port"],
    version: 2,
    is_default: false,
    domain_count: 0,
    created_at: new Date(Date.now() - 20 * 86400 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 5 * 86400 * 1000).toISOString(),
  },
];

const getInitialVhostVersions = (): MockVhostVersion[] => [
  {
    id: "ver_03",
    template_id: "tpl_01",
    version: 3,
    content: `server {
    listen 80;
    listen [::]:80;
    server_name {{domain}} www.{{domain}};
    root {{docroot}};
    index index.php index.html index.htm;

    access_log /var/log/nginx/{{domain}}.access.log;
    error_log /var/log/nginx/{{domain}}.error.log;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \\.php$ {
        fastcgi_pass unix:{{php_socket}};
        fastcgi_index index.php;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        include fastcgi_params;
    }

    location ~ /\\.(?!well-known).* {
        deny all;
    }
}`,
    changelog: "Add hidden files protection except for well-known ACME challenges",
    created_by: "admin",
    created_at: new Date(Date.now() - 2 * 86400 * 1000).toISOString(),
  },
  {
    id: "ver_02",
    template_id: "tpl_01",
    version: 2,
    content: `server {
    listen 80;
    server_name {{domain}} www.{{domain}};
    root {{docroot}};
    index index.php index.html;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location ~ \\.php$ {
        fastcgi_pass unix:{{php_socket}};
        include fastcgi_params;
    }
}`,
    changelog: "Support front-controller routing with try_files query parameter pass-through",
    created_by: "admin",
    created_at: new Date(Date.now() - 15 * 86400 * 1000).toISOString(),
  },
  {
    id: "ver_01",
    template_id: "tpl_01",
    version: 1,
    content: `server {
    listen 80;
    server_name {{domain}};
    root {{docroot}};
    index index.php index.html;

    location ~ \\.php$ {
        fastcgi_pass unix:{{php_socket}};
    }
}`,
    changelog: "Initial basic PHP vhost configuration",
    created_by: "admin",
    created_at: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
  },
];

let mockVhostTemplates = getInitialVhostTemplates();
let mockVhostVersions = getInitialVhostVersions();

export const resetMockVhost = () => {
  mockVhostTemplates = getInitialVhostTemplates();
  mockVhostVersions = getInitialVhostVersions();
};

const defaultPlanIni = {
  memory_limit: "256M",
  upload_max_filesize: "64M",
  post_max_size: "64M",
  max_execution_time: 60,
  max_input_time: 60,
  display_errors: false,
  date_timezone: "UTC",
};

const getInitialPhpConfig = () => ({
  default_version: "8.3",
  supported_versions: [
    { version: "8.4", status: "active" as const, is_default: false, socket_path: "/run/php/php8.4-fpm.sock" },
    { version: "8.3", status: "active" as const, is_default: true, socket_path: "/run/php/php8.3-fpm.sock" },
    { version: "8.2", status: "active" as const, is_default: false, socket_path: "/run/php/php8.2-fpm.sock" },
    { version: "8.1", status: "deprecated" as const, is_default: false, socket_path: "/run/php/php8.1-fpm.sock" },
  ],
  domain_configs: [
    { domain_id: "dom_01", domain_name: "alpha-hosting.com", php_version: "8.3", is_custom: false },
    { domain_id: "dom_02", domain_name: "blog.alpha-hosting.com", php_version: "8.2", is_custom: true },
  ],
  ini_settings: { ...defaultPlanIni },
  plan_defaults: { ...defaultPlanIni },
  plan_limits: {
    max_memory_limit: "512M",
    max_execution_time: 300,
    max_upload_filesize: "128M",
  },
});

let mockClientPhpConfig = getInitialPhpConfig();

export const resetMockPhp = () => {
  mockClientPhpConfig = getInitialPhpConfig();
};

interface MockService {
  id: string;
  name: "nginx" | "php-fpm";
  display_name: string;
  description: string;
  status: "running" | "stopped" | "restarting" | "degraded";
  uptime_seconds: number;
  memory_usage_mb: number;
  cpu_percent: number;
  workers_count: number;
  last_restarted_at: string;
  restarts_limit_per_minute: number;
}

const getInitialServices = (): MockService[] => [
  {
    id: "srv_nginx",
    name: "nginx",
    display_name: "Nginx Web Server",
    description: "Reverse proxy, static asset delivery, and TLS termination engine",
    status: "running",
    uptime_seconds: 84600,
    memory_usage_mb: 48,
    cpu_percent: 0.8,
    workers_count: 4,
    last_restarted_at: new Date(Date.now() - 86400 * 1000).toISOString(),
    restarts_limit_per_minute: 3,
  },
  {
    id: "srv_php_fpm",
    name: "php-fpm",
    display_name: "PHP-FPM Process Manager",
    description: "FastCGI process pool serving dynamic tenant PHP applications",
    status: "running",
    uptime_seconds: 43200,
    memory_usage_mb: 136,
    cpu_percent: 1.4,
    workers_count: 6,
    last_restarted_at: new Date(Date.now() - 43200 * 1000).toISOString(),
    restarts_limit_per_minute: 3,
  },
];

let mockServices = getInitialServices();
let mockServiceRestartsLog: Record<string, number[]> = {
  nginx: [],
  "php-fpm": [],
};

export const resetMockServices = () => {
  mockServices = getInitialServices();
  mockServiceRestartsLog = {
    nginx: [],
    "php-fpm": [],
  };
};

interface MockPageTemplate {
  id: string;
  type: "suspended_domain" | "suspended_user" | "error_404" | "error_502";
  title: string;
  description: string;
  statusCode: number;
  content: string;
  defaultContent: string;
  updatedAt: string;
  isCustomized: boolean;
  variables: { name: string; description: string; sample: string }[];
}

const defaultSuspendedDomainHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Domain Suspended - {{domain}}</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: sans-serif; background-color: #0f172a; color: #f8fafc; min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 1.5rem; }
    .card { background: #1e293b; border: 1px solid #334155; border-radius: 1rem; max-width: 540px; width: 100%; padding: 2.5rem; text-align: center; }
    .badge { display: inline-block; padding: 0.35rem 0.85rem; border-radius: 9999px; font-size: 0.85rem; font-weight: 600; background: #7f1d1d; color: #fecaca; margin-bottom: 1.25rem; }
    h1 { font-size: 1.75rem; font-weight: 700; color: #f1f5f9; margin-bottom: 0.75rem; }
    p { color: #94a3b8; font-size: 0.95rem; line-height: 1.6; margin-bottom: 1.5rem; }
    .domain-box { background: #0f172a; border: 1px solid #334155; border-radius: 0.5rem; padding: 0.85rem; font-family: monospace; color: #38bdf8; font-size: 1rem; margin-bottom: 1.5rem; }
    .reason-box { background: rgba(239, 68, 68, 0.1); border-left: 3px solid #ef4444; padding: 0.75rem 1rem; text-align: left; border-radius: 0.25rem; color: #fca5a5; font-size: 0.875rem; margin-bottom: 1.75rem; }
    .btn { display: inline-block; background: #2563eb; color: #ffffff; text-decoration: none; font-weight: 600; padding: 0.75rem 1.5rem; border-radius: 0.5rem; }
    .footer { margin-top: 2rem; font-size: 0.75rem; color: #64748b; }
  </style>
</head>
<body>
  <div class="card">
    <div class="badge">Notice</div>
    <h1>Domain Suspended</h1>
    <p>This domain name has been suspended and its services are temporarily disabled.</p>
    <div class="domain-box">{{domain}}</div>
    <div class="reason-box"><strong>Reason:</strong> {{reason}}</div>
    <a href="mailto:{{contact_email}}" class="btn">Contact Administrator</a>
    <div class="footer">&copy; {{year}} {{panel_name}}. All rights reserved.</div>
  </div>
</body>
</html>`;

const defaultSuspendedUserHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Account Suspended</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: sans-serif; background-color: #0f172a; color: #f8fafc; min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 1.5rem; }
    .card { background: #1e293b; border: 1px solid #334155; border-radius: 1rem; max-width: 540px; width: 100%; padding: 2.5rem; text-align: center; }
    h1 { font-size: 1.75rem; font-weight: 700; color: #f1f5f9; margin-bottom: 0.75rem; }
    p { color: #94a3b8; font-size: 0.95rem; line-height: 1.6; margin-bottom: 1.5rem; }
    .btn { display: inline-block; background: #2563eb; color: #ffffff; text-decoration: none; font-weight: 600; padding: 0.75rem 1.5rem; border-radius: 0.5rem; }
    .footer { margin-top: 2rem; font-size: 0.75rem; color: #64748b; }
  </style>
</head>
<body>
  <div class="card">
    <h1>Account Suspended</h1>
    <p>The hosting account associated with this site is currently suspended.</p>
    <a href="mailto:{{contact_email}}" class="btn">Contact Support</a>
    <div class="footer">&copy; {{year}} {{panel_name}}. All rights reserved.</div>
  </div>
</body>
</html>`;

const defaultError404Html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>404 - Page Not Found</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: sans-serif; background-color: #0b0f19; color: #f3f4f6; min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 1.5rem; }
    .container { text-align: center; max-width: 500px; width: 100%; }
    .code { font-size: 6rem; font-weight: 900; line-height: 1; color: #38bdf8; margin-bottom: 0.5rem; }
    h1 { font-size: 1.75rem; font-weight: 700; margin-bottom: 0.75rem; color: #f9fafb; }
    p { color: #9ca3af; font-size: 1rem; line-height: 1.6; margin-bottom: 1.75rem; }
    .path-box { display: inline-block; background: #1f2937; border-radius: 0.375rem; padding: 0.35rem 0.75rem; font-family: monospace; color: #e5e7eb; font-size: 0.875rem; margin-bottom: 1.5rem; }
    .footer { margin-top: 2.5rem; font-size: 0.75rem; color: #4b5563; }
  </style>
</head>
<body>
  <div class="container">
    <div class="code">404</div>
    <h1>Page Not Found</h1>
    <p>The page or resource you requested could not be located on this server.</p>
    <div class="path-box">{{path}}</div>
    <div class="footer">&copy; {{year}} {{panel_name}} Web Services</div>
  </div>
</body>
</html>`;

const defaultError502Html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>502 - Bad Gateway</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: sans-serif; background-color: #0b0f19; color: #f3f4f6; min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 1.5rem; }
    .card { background: #111827; border: 1px solid #1f2937; border-radius: 0.75rem; max-width: 580px; width: 100%; padding: 2.5rem; text-align: center; }
    .code { font-size: 4rem; font-weight: 800; color: #f87171; line-height: 1; margin-bottom: 0.5rem; }
    h1 { font-size: 1.5rem; font-weight: 700; color: #f9fafb; margin-bottom: 0.75rem; }
    p { color: #9ca3af; font-size: 0.95rem; line-height: 1.6; margin-bottom: 1.5rem; }
    .footer { font-size: 0.75rem; color: #6b7280; }
  </style>
</head>
<body>
  <div class="card">
    <div class="code">502</div>
    <h1>Bad Gateway</h1>
    <p>The web server received an invalid response while acting as a gateway or proxy to the upstream PHP application ({{service}}).</p>
    <div class="footer">&copy; {{year}} {{panel_name}} Web Services</div>
  </div>
</body>
</html>`;

const getInitialPageTemplates = (): MockPageTemplate[] => [
  {
    id: "tpl_suspended_domain",
    type: "suspended_domain",
    title: "Domain Suspended",
    description: "Served when a specific domain is suspended by the administrator.",
    statusCode: 451,
    content: defaultSuspendedDomainHtml,
    defaultContent: defaultSuspendedDomainHtml,
    updatedAt: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Suspended domain name", sample: "example.com" },
      { name: "{{reason}}", description: "Reason for domain suspension", sample: "Terms of service review" },
      { name: "{{contact_email}}", description: "Support or admin contact email", sample: "admin@builtpanel.net" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_suspended_user",
    type: "suspended_user",
    title: "Account Suspended",
    description: "Served for all sites belonging to a suspended tenant user account.",
    statusCode: 403,
    content: defaultSuspendedUserHtml,
    defaultContent: defaultSuspendedUserHtml,
    updatedAt: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
    isCustomized: false,
    variables: [
      { name: "{{username}}", description: "Suspended user account name", sample: "tenant_user" },
      { name: "{{domain}}", description: "Domain name being accessed", sample: "client-site.com" },
      { name: "{{contact_email}}", description: "Administrative support email", sample: "support@builtpanel.net" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_error_404",
    type: "error_404",
    title: "404 Not Found",
    description: "Custom 404 page displayed when requested route or file does not exist.",
    statusCode: 404,
    content: defaultError404Html,
    defaultContent: defaultError404Html,
    updatedAt: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Host domain name", sample: "my-app.net" },
      { name: "{{path}}", description: "Missing request path", sample: "/old-missing-page" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_error_502",
    type: "error_502",
    title: "502 Bad Gateway",
    description: "Custom 502 error page displayed when upstream backend or PHP-FPM daemon fails.",
    statusCode: 502,
    content: defaultError502Html,
    defaultContent: defaultError502Html,
    updatedAt: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Host domain name", sample: "api-server.org" },
      { name: "{{service}}", description: "Failing upstream daemon name", sample: "php-fpm-8.3" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
];

let mockPageTemplates = getInitialPageTemplates();

export const resetMockPageTemplates = () => {
  mockPageTemplates = getInitialPageTemplates();
};

interface MockPreviewLink {
  domain_id: string;
  target_domain: string;
  enabled: boolean;
  token: string;
  url: string;
  created_at: string;
  expires_at: string;
  ttl_hours: number;
}

const getInitialPreviewLinks = (): Record<string, MockPreviewLink> => ({
  dom_02: {
    domain_id: "dom_02",
    target_domain: "blog.alpha-hosting.com",
    enabled: true,
    token: "exp_test_7890",
    url: "https://exp_test_7890.preview.builtpanel.net",
    created_at: new Date(Date.now() - 48 * 3600 * 1000).toISOString(),
    expires_at: new Date(Date.now() - 24 * 3600 * 1000).toISOString(), // Expired
    ttl_hours: 24,
  },
});

let mockPreviewLinks = getInitialPreviewLinks();

export const resetMockPreviewLinks = () => {
  mockPreviewLinks = getInitialPreviewLinks();
};

// The shapes of `DatabaseView` and `DbUserView` in openapi.json.
interface MockDb {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  status: "pending" | "ready" | "failed" | "deleting";
  size_bytes: number;
  size_checked_at: string | null;
  users: { user: string; user_id: string; access: string }[];
}

interface MockDbUser {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  status: string;
  databases: { database: string; database_id: string; access: string }[];
  remote_hosts: string[];
}

const getInitialDatabases = (): MockDb[] => [
  {
    id: "db_01",
    name: "demo_wordpress",
    engine: "mysql",
    status: "ready",
    size_bytes: 24657920,
    size_checked_at: "2026-11-10T12:00:00Z",
    users: [{ user: "demo_wpuser", user_id: "dbusr_01", access: "all" }],
  },
  {
    id: "db_02",
    name: "demo_analytics",
    engine: "mysql",
    status: "ready",
    size_bytes: 19922944,
    size_checked_at: "2026-11-11T15:30:00Z",
    users: [{ user: "demo_analyst", user_id: "dbusr_02", access: "read_write" }],
  },
];

const getInitialDbUsers = (): MockDbUser[] => [
  {
    id: "dbusr_01",
    name: "demo_wpuser",
    engine: "mysql",
    status: "ready",
    databases: [{ database: "demo_wordpress", database_id: "db_01", access: "all" }],
    remote_hosts: ["198.51.100.12/32"],
  },
  {
    id: "dbusr_02",
    name: "demo_analyst",
    engine: "mysql",
    status: "ready",
    databases: [{ database: "demo_analytics", database_id: "db_02", access: "read_write" }],
    remote_hosts: [],
  },
];

let mockDatabases = getInitialDatabases();
let mockDbUsers = getInitialDbUsers();

export const resetMockDatabases = () => {
  mockDatabases = getInitialDatabases();
  mockDbUsers = getInitialDbUsers();
};


const mockMe = {
  id: "00000000-0000-0000-0000-000000000001",
  username: "admin",
  email: "admin@builtpanel.local",
  kind: "admin",
  locale: "bn",
  mfa_enabled: false,
  permissions: ["*"],
  roles: ["super_admin"],
  modules: [
    "dashboard",
    "security",
    "users",
    "nodes",
    "operations",
    "feature-sets",
    "plans",
    "audit",
    "domains",
    "dns",
    "ssl",
    "settings",
    "demo",
  ],
  features: ["all"],
  csrf_token: "mock-csrf-token-admin-12345",
};

// An operation as the backend's `OperationView` has it.
const operationView = (o: (typeof mockOperations)[number]) => ({
  id: o.id,
  idempotency_key: o.idempotency_key,
  kind: o.op_type,
  node_id: o.node_id,
  tenant_id: o.tenant_id ?? null,
  status: o.status,
  attempts: o.retries_count + 1,
  max_attempts: 5,
  payload: o.payload,
  output: o.output ?? null,
  error: o.error ? { code: o.error.code, message: o.error.message, retryable: true } : null,
  created_at: o.created_at,
  dispatched_at: null,
  finished_at: o.status === "succeeded" || o.status === "failed" ? o.updated_at : null,
  next_attempt_at: o.updated_at,
  expires_at: o.updated_at,
});

export const handlers = [
  // Health
  http.get(/\/api\/v1\/public\/health$/, () => {
    return HttpResponse.json({
      status: "ok",
      version: "0.1.0",
      timestamp: new Date().toISOString(),
    });
  }),

  // Auth: Current user (GET /api/v1/auth/me)
  http.get(/\/api\/v1\/auth\/me$/, () => {
    return HttpResponse.json({
      ...mockMe,
      mfa_enabled: mfaEnabled,
    });
  }),

  // GET /api/v1/admin/bootstrap and /api/v1/client/bootstrap, shaped like `Bootstrap` in openapi.json
  http.get(/\/api\/v1\/admin\/bootstrap$/, () => {
    return HttpResponse.json({
      user: { id: mockMe.id, username: mockMe.username, email: mockMe.email, kind: "admin" },
      roles: ["super_admin"],
      permissions: ["*"],
      mfa: { enabled: mfaEnabled, required: false, setup_required: false },
      modules: mockMe.modules.map((name) => ({
        name,
        version: "0.1.0",
        title: { en: name },
        category: "core",
        core: true,
        nav: { group: "System", order: 10, icon: "blocks" },
      })),
      features: [],
      limits: {},
      branding: { panel_name: "Built Panel", maintenance_notice: "", default_language: "en" },
      locale: "bn",
      csrf_token: mockMe.csrf_token,
    });
  }),

  http.get(/\/api\/v1\/client\/bootstrap$/, () => {
    return HttpResponse.json({
      user: { id: "00000000-0000-0000-0000-000000000002", username: "demo_user", email: "demo@example.com", kind: "client" },
      roles: ["client"],
      permissions: [],
      mfa: { enabled: false, required: false, setup_required: false },
      modules: ["domains", "files", "databases", "cron", "backups", "logs"].map((name) => ({
        name,
        version: "0.1.0",
        title: { en: name },
        category: "web",
        core: false,
        nav: { group: "Web", order: 10, icon: "globe" },
      })),
      features: ["domains", "redirects", "dns", "ssl", "files", "ftp", "databases", "backups", "cron", "logs"],
      limits: { max_domains: 5, max_databases: 5, max_cron_jobs: 10 },
      branding: { panel_name: "Built Panel", maintenance_notice: "", default_language: "en" },
      locale: "bn",
      csrf_token: "mock-csrf-token-client-12345",
    });
  }),

  // Auth: Login step 1 (POST /api/v1/public/auth/login)
  http.post(/\/api\/v1\/public\/auth\/login$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const login = body?.login || body?.username;
    const password = body?.password;

    if (!login || !password) {
      return HttpResponse.json(
        { title: "Invalid username or password", status: 401 },
        { status: 401 }
      );
    }

    if (login === "invalid" || password === "wrong") {
      return HttpResponse.json(
        { title: "Invalid username or password", status: 401 },
        { status: 401 }
      );
    }

    if (login === "admin_2fa") {
      return HttpResponse.json(
        {
          mfa_required: true,
          methods: ["totp"],
          expires_in: 300,
        },
        { status: 202 }
      );
    }

    return HttpResponse.json({
      ...mockMe,
      username: login,
      email: `${login}@builtpanel.local`,
    });
  }),

  // Auth: Login step 2 (POST /api/v1/public/auth/login/mfa)
  http.post(/\/api\/v1\/public\/auth\/login\/mfa$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const { code } = body || {};

    if (code !== "123456") {
      return HttpResponse.json(
        { title: "Invalid MFA code", status: 401 },
        { status: 401 }
      );
    }

    return HttpResponse.json({
      ...mockMe,
      username: "admin_2fa",
      email: "admin_2fa@builtpanel.local",
      mfa_enabled: true,
    });
  }),

  // Auth: Logout (POST /api/v1/auth/logout)
  http.post(/\/api\/v1\/auth\/logout$/, () => {
    return new HttpResponse(null, { status: 204 });
  }),

  // Auth: Sudo mode (POST /api/v1/auth/sudo)
  http.post(/\/api\/v1\/auth\/sudo$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    if (body?.password === "wrong-password") {
      return HttpResponse.json(
        { title: "Incorrect password", status: 401 },
        { status: 401 }
      );
    }
    return new HttpResponse(null, { status: 204 });
  }),

  // MFA: Status (GET /api/v1/auth/mfa)
  http.get(/\/api\/v1\/auth\/mfa$/, () => {
    return HttpResponse.json({
      mfa_enabled: mfaEnabled,
      backup_codes_remaining: mfaEnabled ? backupCodes.length : 0,
    });
  }),

  // MFA: Enroll TOTP (POST /api/v1/auth/mfa/totp/enroll)
  http.post(/\/api\/v1\/auth\/mfa\/totp\/enroll$/, () => {
    return HttpResponse.json({
      secret: "JBSWY3DPEHPK3PXP",
      otpauth_uri: "otpauth://totp/Built%20Panel:admin@builtpanel.local?secret=JBSWY3DPEHPK3PXP&issuer=Built%20Panel",
    });
  }),

  // MFA: Confirm TOTP (POST /api/v1/auth/mfa/totp/confirm)
  http.post(/\/api\/v1\/auth\/mfa\/totp\/confirm$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const code = body?.code || body?.totp_code;

    if (code !== "123456") {
      return HttpResponse.json(
        { title: "Invalid 6-digit TOTP code", status: 400 },
        { status: 400 }
      );
    }

    mfaEnabled = true;
    return HttpResponse.json({
      backup_codes: backupCodes,
    });
  }),

  // MFA: Disable TOTP (POST /api/v1/auth/mfa/totp/disable)
  http.post(/\/api\/v1\/auth\/mfa\/totp\/disable$/, () => {
    mfaEnabled = false;
    return new HttpResponse(null, { status: 204 });
  }),

  // Sessions: List live sessions (GET /api/v1/auth/sessions)
  http.get(/\/api\/v1\/auth\/sessions$/, () => {
    if (sessions.length <= 1) {
      sessions = getInitialSessions();
    }
    return HttpResponse.json({ sessions });
  }),

  // Sessions: Revoke a session (DELETE /api/v1/auth/sessions/:id)
  http.delete(/\/api\/v1\/auth\/sessions\/(.+)$/, ({ params }) => {
    const id = params[0];
    sessions = sessions.filter((s) => s.id !== id);
    return new HttpResponse(null, { status: 204 });
  }),

  // Sessions: Revoke all other sessions (POST /api/v1/auth/sessions/revoke-others)
  http.post(/\/api\/v1\/auth\/sessions\/revoke-others$/, () => {
    const initialCount = sessions.length;
    sessions = sessions.filter((s) => s.current);
    return HttpResponse.json({
      revoked_count: Math.max(0, initialCount - sessions.length),
    });
  }),

  // Audit Logs (GET /api/v1/admin/audit-logs) (T01.12 / T01.16)
  ...(() => {
    const mockAuditLogs = Array.from({ length: 30 }, (_, i) => {
      const actions = [
        "auth.login",
        "auth.login_failed",
        "auth.mfa_enrolled",
        "auth.session_revoked",
        "auth.password_changed",
        "auth.sudo_granted",
        "vhost.create",
        "domain.create",
        "user.role_assigned",
      ];
      const outcomes = ["success", "failure", "denied"] as const;
      const targetTypes = ["session", "account", "domain", "role"];
      const action = actions[i % actions.length];
      const outcome = action.includes("failed") ? "failure" : outcomes[i % outcomes.length];
      const targetType = targetTypes[i % targetTypes.length];
      const at = new Date(Date.now() - (i + 1) * 3600 * 1000 * 2).toISOString();

      return {
        id: `00000000-0000-0000-0000-${String(i + 1).padStart(12, "0")}`,
        at,
        action,
        outcome,
        actor_id: i % 3 === 0 ? "00000000-0000-0000-0000-000000000001" : `00000000-0000-0000-0000-${String(i + 10).padStart(12, "0")}`,
        actor_ip: i % 2 === 0 ? "192.168.1.105" : "203.0.113.42",
        correlation_id: `req-${1000 + i}-trace`,
        node_id: "00000000-0000-0000-0000-0000000000aa",
        target_type: targetType,
        target_id: `obj-${100 + i}`,
        tenant_id: i % 4 === 0 ? "00000000-0000-0000-0000-000000000001" : null,
        metadata: {
          client: i % 2 === 0 ? "Chrome 130 (macOS)" : "Firefox 131 (Linux)",
          details: `Executed ${action} with status ${outcome}`,
          ip: i % 2 === 0 ? "192.168.1.105" : "203.0.113.42",
          duration_ms: (i + 1) * 12,
        },
      };
    });

    return [
      http.get(/\/api\/v1\/admin\/audit-logs/, ({ request }) => {
        const url = new URL(request.url);
        const action = url.searchParams.get("action");
        const actorId = url.searchParams.get("actor_id");
        const outcome = url.searchParams.get("outcome");
        const targetType = url.searchParams.get("target_type");
        const targetId = url.searchParams.get("target_id");
        const limit = parseInt(url.searchParams.get("limit") || "10", 10);
        const cursor = url.searchParams.get("cursor");

        let filtered = [...mockAuditLogs];

        if (action) {
          filtered = filtered.filter((item) =>
            item.action.toLowerCase().startsWith(action.toLowerCase())
          );
        }
        if (actorId) {
          filtered = filtered.filter((item) => item.actor_id === actorId);
        }
        if (outcome && outcome !== "all") {
          filtered = filtered.filter((item) => item.outcome === outcome);
        }
        if (targetType) {
          filtered = filtered.filter((item) => item.target_type === targetType);
        }
        if (targetId) {
          filtered = filtered.filter((item) => item.target_id === targetId);
        }

        let startIndex = 0;
        if (cursor) {
          const found = filtered.findIndex((item) => item.id === cursor);
          if (found !== -1) {
            startIndex = found + 1;
          }
        }

        const pageItems = filtered.slice(startIndex, startIndex + limit);
        const nextItem = filtered[startIndex + limit];
        const nextCursor = nextItem ? nextItem.id : null;

        return HttpResponse.json({
          items: pageItems,
          next_cursor: nextCursor,
        });
      }),
    ];
  })(),

  // Node Management (GET /api/v1/admin/nodes) TODO(api:T02.08)
  ...(() => {
    const mockNodes = [
      {
        id: "00000000-0000-0000-0000-0000000000aa",
        name: "node-master-01",
        hostname: "cp1.builtpanel.net",
        ip_address: "167.104.101.81",
        status: "online",
        version: "0.1.0",
        fingerprint: "SHA256:7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b",
        last_seen_at: new Date().toISOString(),
        enrolled_at: new Date(Date.now() - 7 * 86400 * 1000).toISOString(),
        capabilities: {
          cpu_cores: 8,
          cpu_model: "AMD EPYC 7763 64-Core Processor",
          cpu_usage_pct: 18,
          ram_total_bytes: 16 * 1024 * 1024 * 1024,
          ram_used_bytes: 5.4 * 1024 * 1024 * 1024,
          disk_total_bytes: 250 * 1024 * 1024 * 1024,
          disk_used_bytes: 52 * 1024 * 1024 * 1024,
          kernel: "Linux 6.8.0-45-generic",
          docker_version: "27.3.1",
          security_features: ["seccomp", "apparmor", "userns"],
        },
        capacity: {
          allocated_cpu_cores: 12,
          allocated_ram_bytes: 18 * 1024 * 1024 * 1024,
          allocated_disk_bytes: 160 * 1024 * 1024 * 1024,
          tenants_count: 24,
          max_tenants: 50,
          saturation_pct: 68,
          oversell: {
            cpu_ratio: 2.0,
            ram_ratio: 1.25,
            disk_ratio: 1.0,
          },
        },
      },
      {
        id: "00000000-0000-0000-0000-0000000000bb",
        name: "node-worker-02",
        hostname: "worker2.builtpanel.net",
        ip_address: "167.104.101.82",
        status: "draining",
        version: "0.1.0",
        fingerprint: "SHA256:9f8e7d6c5b4a3f2e1d0c9b8a7f6e5d4c3b2a1f0e",
        last_seen_at: new Date(Date.now() - 30 * 1000).toISOString(),
        enrolled_at: new Date(Date.now() - 3 * 86400 * 1000).toISOString(),
        capabilities: {
          cpu_cores: 16,
          cpu_model: "AMD EPYC 7763 64-Core Processor",
          cpu_usage_pct: 42,
          ram_total_bytes: 32 * 1024 * 1024 * 1024,
          ram_used_bytes: 18.2 * 1024 * 1024 * 1024,
          disk_total_bytes: 500 * 1024 * 1024 * 1024,
          disk_used_bytes: 140 * 1024 * 1024 * 1024,
          kernel: "Linux 6.8.0-45-generic",
          docker_version: "27.3.1",
          security_features: ["seccomp", "apparmor"],
        },
        capacity: {
          allocated_cpu_cores: 28,
          allocated_ram_bytes: 34 * 1024 * 1024 * 1024,
          allocated_disk_bytes: 420 * 1024 * 1024 * 1024,
          tenants_count: 48,
          max_tenants: 60,
          saturation_pct: 85,
          oversell: {
            cpu_ratio: 2.0,
            ram_ratio: 1.25,
            disk_ratio: 1.0,
          },
        },
      },
      {
        id: "00000000-0000-0000-0000-0000000000cc",
        name: "node-worker-03-backup",
        hostname: "worker3.builtpanel.net",
        ip_address: "167.104.101.83",
        status: "offline",
        version: "0.1.0",
        fingerprint: "SHA256:1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b",
        last_seen_at: new Date(Date.now() - 3600 * 1000 * 5).toISOString(),
        enrolled_at: new Date(Date.now() - 14 * 86400 * 1000).toISOString(),
        capabilities: {
          cpu_cores: 4,
          cpu_model: "Intel Xeon E-2288G CPU @ 3.70GHz",
          cpu_usage_pct: 0,
          ram_total_bytes: 8 * 1024 * 1024 * 1024,
          ram_used_bytes: 1.1 * 1024 * 1024 * 1024,
          disk_total_bytes: 120 * 1024 * 1024 * 1024,
          disk_used_bytes: 22 * 1024 * 1024 * 1024,
          kernel: "Linux 6.5.0-28-generic",
          docker_version: "26.1.4",
          security_features: ["seccomp"],
        },
        capacity: {
          allocated_cpu_cores: 2,
          allocated_ram_bytes: 2 * 1024 * 1024 * 1024,
          allocated_disk_bytes: 25 * 1024 * 1024 * 1024,
          tenants_count: 4,
          max_tenants: 20,
          saturation_pct: 22,
          oversell: {
            cpu_ratio: 2.0,
            ram_ratio: 1.25,
            disk_ratio: 1.0,
          },
        },
      },
    ];

    const mockEnrollments: any[] = [
      {
        id: "00000000-0000-0000-0000-000000000001",
        name: "node-worker-pending",
        created_at: new Date(Date.now() - 300 * 1000).toISOString(),
        created_by: "00000000-0000-0000-0000-000000000001",
        expires_at: new Date(Date.now() + 600 * 1000).toISOString(),
        status: "pending",
      },
    ];

    interface MockNodeIp {
      id: string;
      node_id: string;
      node_name: string;
      ip_address: string;
      subnet_mask: string;
      interface_name: string;
      type: "shared" | "dedicated";
      status: "active" | "available" | "reserved";
      is_primary: boolean;
      assigned_tenant_id?: string | null;
      assigned_tenant_username?: string | null;
      assigned_domain?: string | null;
      shared_tenants_count?: number;
      created_at: string;
      updated_at: string;
    }

    const subnetMaskTrim = (mask?: string) => (mask ? mask.trim() : "255.255.255.0");

    const initialMockIps: MockNodeIp[] = [
      {
        id: "ip_01",
        node_id: "00000000-0000-0000-0000-0000000000aa",
        node_name: "node-master-01",
        ip_address: "167.104.101.81",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0",
        type: "shared",
        status: "active",
        is_primary: true,
        shared_tenants_count: 24,
        created_at: new Date(Date.now() - 30 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 30 * 86400000).toISOString(),
      },
      {
        id: "ip_02",
        node_id: "00000000-0000-0000-0000-0000000000aa",
        node_name: "node-master-01",
        ip_address: "167.104.101.82",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0",
        type: "shared",
        status: "active",
        is_primary: false,
        shared_tenants_count: 10,
        created_at: new Date(Date.now() - 20 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 20 * 86400000).toISOString(),
      },
      {
        id: "ip_03",
        node_id: "00000000-0000-0000-0000-0000000000aa",
        node_name: "node-master-01",
        ip_address: "167.104.101.95",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0:1",
        type: "dedicated",
        status: "active",
        is_primary: false,
        assigned_tenant_id: "usr_01",
        assigned_tenant_username: "alex",
        assigned_domain: "mail.alexblog.org",
        created_at: new Date(Date.now() - 10 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 2 * 86400000).toISOString(),
      },
      {
        id: "ip_04",
        node_id: "00000000-0000-0000-0000-0000000000aa",
        node_name: "node-master-01",
        ip_address: "167.104.101.96",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0:2",
        type: "dedicated",
        status: "available",
        is_primary: false,
        created_at: new Date(Date.now() - 5 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 5 * 86400000).toISOString(),
      },
      {
        id: "ip_05",
        node_id: "00000000-0000-0000-0000-0000000000bb",
        node_name: "node-worker-02",
        ip_address: "167.104.101.85",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0",
        type: "shared",
        status: "active",
        is_primary: true,
        shared_tenants_count: 8,
        created_at: new Date(Date.now() - 15 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 15 * 86400000).toISOString(),
      },
      {
        id: "ip_06",
        node_id: "00000000-0000-0000-0000-0000000000bb",
        node_name: "node-worker-02",
        ip_address: "167.104.101.99",
        subnet_mask: "255.255.255.0",
        interface_name: "eth0:1",
        type: "dedicated",
        status: "available",
        is_primary: false,
        created_at: new Date(Date.now() - 8 * 86400000).toISOString(),
        updated_at: new Date(Date.now() - 8 * 86400000).toISOString(),
      },
    ];

    const mockIps = [...initialMockIps];

    return [
      // List Nodes (GET /api/v1/admin/nodes) TODO(api:T02.08)
      // Shaped like `NodeView` in openapi.json
      http.get(/\/api\/v1\/admin\/nodes$/, () => {
        return HttpResponse.json({
          items: mockNodes.map((n) => ({
            id: n.id,
            name: n.name,
            hostname: n.hostname,
            public_ip: n.ip_address,
            status: n.status,
            agent_version: n.version,
            capabilities: {
              cpu_cores: n.capabilities.cpu_cores,
              memory_bytes: n.capabilities.ram_total_bytes,
              disk_total_bytes: n.capabilities.disk_total_bytes,
              disk_free_bytes: n.capabilities.disk_total_bytes - n.capabilities.disk_used_bytes,
              kernel: n.capabilities.kernel,
              docker_version: n.capabilities.docker_version,
              seccomp: n.capabilities.security_features.includes("seccomp"),
              apparmor: n.capabilities.security_features.includes("apparmor"),
              user_namespaces: n.capabilities.security_features.includes("userns"),
            },
            last_seen_at: n.last_seen_at,
            created_at: n.enrolled_at,
          })).concat(
            // The nodes the demo users live on.
            mockNodesList
              .filter((n) => !mockNodes.some((m) => m.id === n.id))
              .map((n) => ({
                id: n.id,
                name: n.name,
                hostname: n.name,
                public_ip: null as unknown as string,
                status: n.status,
                agent_version: "0.1.0",
                capabilities: {} as any,
                last_seen_at: new Date().toISOString(),
                created_at: new Date().toISOString(),
              })),
          ),
        });
      }),

      // Create Enrollment Token (POST /api/v1/admin/nodes)
      http.post(/\/api\/v1\/admin\/nodes$/, async ({ request }) => {
        const body = (await request.json().catch(() => ({}))) as any;
        const name = body?.name || "node-worker-" + Math.floor(Math.random() * 100);
        const id = "00000000-0000-0000-0000-" + Math.random().toString(16).substring(2, 14).padEnd(12, "0");
        const token = "dhn_" + Math.random().toString(36).substring(2, 15) + Math.random().toString(36).substring(2, 15);
        const expiresIn = 900;
        const expiresAt = new Date(Date.now() + expiresIn * 1000).toISOString();
        const enrollCommand = `curl -sSL https://get.builtpanel.com/agent.sh | sudo bash -s -- --token ${token}`;

        const newEnrollment = {
          id,
          name,
          token,
          expires_at: expiresAt,
          expires_in: expiresIn,
          enroll_command: enrollCommand,
        };

        mockEnrollments.unshift({
          id,
          name,
          created_at: new Date().toISOString(),
          created_by: "00000000-0000-0000-0000-000000000001",
          expires_at: expiresAt,
          status: "pending",
        });

        return HttpResponse.json(newEnrollment, { status: 201 });
      }),

      // List Node Enrollments (GET /api/v1/admin/nodes/enrollments)
      http.get(/\/api\/v1\/admin\/nodes\/enrollments$/, () => {
        return HttpResponse.json({
          items: mockEnrollments,
        });
      }),

      // Cancel/Revoke Node Enrollment (DELETE /api/v1/admin/nodes/enrollments/:id)
      http.delete(/\/api\/v1\/admin\/nodes\/enrollments\/([^/]+)$/, ({ params }) => {
        const { 0: id } = params as any;
        const idx = mockEnrollments.findIndex((e) => e.id === id);
        if (idx !== -1) {
          mockEnrollments[idx].status = "revoked";
        }
        return new HttpResponse(null, { status: 204 });
      }),

      // Revoke Node (POST /api/v1/admin/nodes/:id/revoke)
      http.post(/\/api\/v1\/admin\/nodes\/([^/]+)\/revoke$/, ({ params }) => {
        const { 0: nodeId } = params as any;
        const target = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);
        if (target) {
          target.status = "revoked";
        }
        return HttpResponse.json({ node_id: nodeId, certificates_revoked: 1, session_closed: true });
      }),

      // Get Signed Release Manifest (GET /api/v1/admin/cluster/agent/release-manifest) TODO(api:T16.08)
      http.get(/\/api\/v1\/admin\/cluster\/agent\/release-manifest$/, () => {
        return HttpResponse.json({
          version: "0.2.0",
          channel: "stable",
          signing_key_id: "ed25519-release-2026-v1",
          signature: "ed25519:7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b",
          checksum_sha256: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
          release_notes: "Hardened syscall sandbox & signed self-updater (Phase 16)",
        });
      }),

      // Apply Signed Agent Self-Update (POST /api/v1/admin/cluster/nodes/:id/update-agent) TODO(api:T16.08)
      http.post(/\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/update-agent$/, async ({ params, request }) => {
        const { 0: nodeId } = params as any;
        const body = (await request.json().catch(() => ({}))) as any;
        const target = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);

        if (!target) {
          return HttpResponse.json({ error: "node_not_found", message: "Node not found" }, { status: 404 });
        }

        // Signature verification (DoD T16.08: invalid signature rejected)
        if (body.corrupt_signature || body.manifest_signature?.includes("invalid")) {
          return HttpResponse.json(
            {
              error: "invalid_signature",
              message: "ERR_SIGNATURE_MISMATCH: Agent binary signature verification failed. Update rejected.",
            },
            { status: 422 }
          );
        }

        target.version = body.version || "0.2.0";
        return HttpResponse.json({
          success: true,
          version: target.version,
          message: "Agent self-update verified and applied successfully",
        });
      }),

      // Drain Node (POST /api/v1/admin/nodes/:id/drain) TODO(api:T16.01, T16.02)
      http.post(/\/api\/v1\/admin\/nodes\/([^/]+)\/drain$/, ({ params }) => {
        const { 0: nodeId } = params as any;
        const target = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);
        if (target) {
          target.status = "draining";
        }
        return HttpResponse.json({
          success: true,
          status: "draining",
          message: "Node is now draining",
        });
      }),

      // Undrain Node (POST /api/v1/admin/nodes/:id/undrain) TODO(api:T16.01, T16.02)
      http.post(/\/api\/v1\/admin\/nodes\/([^/]+)\/undrain$/, ({ params }) => {
        const { 0: nodeId } = params as any;
        const target = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);
        if (target) {
          target.status = "online";
        }
        return HttpResponse.json({
          success: true,
          status: "online",
          message: "Node restored to service",
        });
      }),

      // Update Node Capacity & Oversell (PUT /api/v1/admin/nodes/:id/capacity) TODO(api:T16.01)
      http.put(/\/api\/v1\/admin\/nodes\/([^/]+)\/capacity$/, async ({ params, request }) => {
        const { 0: nodeId } = params as any;
        const body = (await request.json().catch(() => ({}))) as any;
        const target = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);
        if (target) {
          if (target.capacity && body.oversell) {
            target.capacity.oversell = {
              ...target.capacity.oversell,
              ...body.oversell,
            };
          }
          if (target.capacity && typeof body.max_tenants === "number") {
            target.capacity.max_tenants = body.max_tenants;
          }
        }
        return HttpResponse.json({
          success: true,
          node: target,
        });
      }),

      // Get Cluster Placement Scheduler Config (GET /api/v1/admin/cluster/scheduler) TODO(api:T16.02)
      http.get(/\/api\/v1\/admin\/cluster\/scheduler$/, () => {
        return HttpResponse.json({
          policy: "least-loaded",
          exclude_draining: true,
          fallback_policy: "round-robin",
        });
      }),

      // Update Cluster Placement Scheduler Config (PUT /api/v1/admin/cluster/scheduler) TODO(api:T16.02)
      http.put(/\/api\/v1\/admin\/cluster\/scheduler$/, async ({ request }) => {
        const body = (await request.json().catch(() => ({}))) as any;
        return HttpResponse.json({
          success: true,
          policy: body.policy || "least-loaded",
          exclude_draining: true,
        });
      }),
      // Cluster IP Pools: List all cluster IP addresses & metrics (GET /api/v1/admin/cluster/ips)
      // TODO(api:T16.05): Replace with real backend cluster IP pool endpoint
      http.get(/\/api\/v1\/admin\/cluster\/ips$/, () => {
        const total_ips = mockIps.length;
        const shared_ips = mockIps.filter((i) => i.type === "shared").length;
        const dedicated_total = mockIps.filter((i) => i.type === "dedicated").length;
        const dedicated_assigned = mockIps.filter(
          (i) => i.type === "dedicated" && i.assigned_tenant_id
        ).length;
        const dedicated_available = dedicated_total - dedicated_assigned;

        return HttpResponse.json({
          items: mockIps,
          metrics: {
            total_ips,
            shared_ips,
            dedicated_total,
            dedicated_assigned,
            dedicated_available,
          },
        });
      }),

      // Node IP Pool: List IP addresses for a node (GET /api/v1/admin/cluster/nodes/:nodeId/ips)
      // TODO(api:T16.05): Replace with real backend node IP pool endpoint
      http.get(/\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/ips$/, ({ params }) => {
        const nodeId = (params.nodeId || params[0]) as string;
        const filtered = mockIps.filter(
          (i) => i.node_id === nodeId || i.node_name === nodeId
        );
        return HttpResponse.json({ items: filtered, total: filtered.length });
      }),

      // Node IP Pool: Add IP address to node (POST /api/v1/admin/cluster/nodes/:nodeId/ips)
      // TODO(api:T16.05): Replace with real backend node IP pool endpoint
      http.post(
        /\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/ips$/,
        async ({ params, request }) => {
          const nodeId = (params.nodeId || params[0]) as string;
          const body = (await request.json().catch(() => ({}))) as any;
          const {
            ip_address,
            subnet_mask = "255.255.255.0",
            interface_name = "eth0",
            type = "dedicated",
          } = body || {};

          if (!ip_address) {
            return HttpResponse.json({ message: "IP address is required" }, { status: 400 });
          }

          const targetNode = mockNodes.find(
            (n) => n.id === nodeId || n.name === nodeId
          );
          const nodeName = targetNode ? targetNode.name : "node-master-01";

          const newIp: MockNodeIp = {
            id: `ip_${Date.now()}`,
            node_id: targetNode ? targetNode.id : nodeId,
            node_name: nodeName,
            ip_address: ip_address.trim(),
            subnet_mask: subnetMaskTrim(subnet_mask),
            interface_name: interface_name.trim() || "eth0",
            type: type === "shared" ? "shared" : "dedicated",
            status: type === "shared" ? "active" : "available",
            is_primary: false,
            shared_tenants_count: type === "shared" ? 0 : undefined,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          };

          mockIps.unshift(newIp);
          return HttpResponse.json(newIp, { status: 201 });
        }
      ),

      // Node IP Pool: Allocate dedicated IP to tenant (POST /api/v1/admin/cluster/nodes/:nodeId/ips/:ipId/assign)
      // TODO(api:T16.05): Replace with real backend IP allocation endpoint
      http.post(
        /\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/ips\/([^/]+)\/assign$/,
        async ({ params, request }) => {
          const ipId = (params.ipId || params[1]) as string;
          const body = (await request.json().catch(() => ({}))) as any;
          const { tenant_id, tenant_username, domain } = body || {};

          const index = mockIps.findIndex((i) => i.id === ipId);
          if (index === -1) {
            return HttpResponse.json({ message: "IP address not found" }, { status: 404 });
          }

          const current = mockIps[index];
          const updated: MockNodeIp = {
            ...current,
            assigned_tenant_id: tenant_id,
            assigned_tenant_username: tenant_username,
            assigned_domain: domain || null,
            status: "active",
            updated_at: new Date().toISOString(),
          };

          mockIps[index] = updated;
          return HttpResponse.json(updated);
        }
      ),

      // Node IP Pool: Release dedicated IP from tenant (POST /api/v1/admin/cluster/nodes/:nodeId/ips/:ipId/release)
      // TODO(api:T16.05): Replace with real backend IP release endpoint
      http.post(
        /\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/ips\/([^/]+)\/release$/,
        ({ params }) => {
          const ipId = (params.ipId || params[1]) as string;
          const index = mockIps.findIndex((i) => i.id === ipId);
          if (index === -1) {
            return HttpResponse.json({ message: "IP address not found" }, { status: 404 });
          }

          const current = mockIps[index];
          const updated: MockNodeIp = {
            ...current,
            assigned_tenant_id: null,
            assigned_tenant_username: null,
            assigned_domain: null,
            status: "available",
            updated_at: new Date().toISOString(),
          };

          mockIps[index] = updated;
          return HttpResponse.json(updated);
        }
      ),

      // Node IP Pool: Delete IP address (DELETE /api/v1/admin/cluster/nodes/:nodeId/ips/:ipId)
      // TODO(api:T16.05): Replace with real backend node IP endpoint
      http.delete(
        /\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/ips\/([^/]+)$/,
        ({ params }) => {
          const ipId = (params.ipId || params[1]) as string;
          const index = mockIps.findIndex((i) => i.id === ipId);
          if (index === -1) {
            return HttpResponse.json({ message: "IP address not found" }, { status: 404 });
          }

          if (mockIps[index].is_primary) {
            return HttpResponse.json(
              { message: "Cannot delete the node's primary IP address" },
              { status: 400 }
            );
          }

          mockIps.splice(index, 1);
          return HttpResponse.json({ success: true });
        }
      ),
      // Failover Offline Node (POST /api/v1/admin/cluster/nodes/:id/failover) TODO(api:T16.07)
      http.post(/\/api\/v1\/admin\/cluster\/nodes\/([^/]+)\/failover$/, async ({ params, request }) => {
        const { 0: nodeId } = params as any;
        const body = (await request.json().catch(() => ({}))) as any;
        const targetNodeId = body.target_node_id;

        const sourceNode = mockNodes.find((n) => n.id === nodeId || n.name === nodeId);
        if (!sourceNode) {
          return HttpResponse.json({ error: "node_not_found", message: "Source node not found" }, { status: 404 });
        }

        const targetNode = mockNodes.find((n) => n.id === targetNodeId || n.name === targetNodeId);
        if (!targetNode) {
          return HttpResponse.json({ error: "target_not_found", message: "Target node not found" }, { status: 404 });
        }

        // Migrate tenants in mockUsers
        let migratedCount = 0;
        mockUsers.forEach((u) => {
          if (
            u.node_id === sourceNode.id ||
            u.node_name === sourceNode.name ||
            (sourceNode.name.includes("backup") && u.node_id === "node-nyc-01")
          ) {
            u.node_id = targetNode.id;
            u.node_name = targetNode.name;
            (u as any).node_status = targetNode.status;
            (u as any).is_node_offline = false;
            migratedCount++;
          }
        });

        // Mark source node status as draining / evacuated
        sourceNode.status = "draining";

        return HttpResponse.json({
          success: true,
          message: `Failover completed successfully. ${migratedCount || 12} tenants evacuated to ${targetNode.name}.`,
          migrated_tenants: migratedCount || 12,
        });
      }),
    ];
  })(),
  // GET /api/v1/admin/operations?status&kind&node_id
  http.get(/\/api\/v1\/admin\/operations$/, ({ request }) => {
    const url = new URL(request.url);
    const status = url.searchParams.get("status");
    const kind = url.searchParams.get("kind");
    const nodeId = url.searchParams.get("node_id");

    let items = [...mockOperations];
    if (status) items = items.filter((op) => op.status === status);
    if (kind) items = items.filter((op) => op.op_type.toLowerCase().includes(kind.toLowerCase()));
    if (nodeId) items = items.filter((op) => op.node_id === nodeId || op.node_name === nodeId);

    return HttpResponse.json({ items: items.map(operationView) });
  }),

  // GET /api/v1/admin/operations/:id
  http.get(/\/api\/v1\/admin\/operations\/([^/]+)$/, ({ params }) => {
    const id = (params.id || (params as any)[0] || (params as any)["0"]) as string;
    const op = mockOperations.find((o) => o.id === id);
    if (op) return HttpResponse.json(operationView(op));
    // Operations other mocks make (a tenant being set up, ...)
    return HttpResponse.json({
      id,
      idempotency_key: id,
      kind: "create_tenant",
      node_id: "node_1",
      status: "succeeded",
      attempts: 1,
      max_attempts: 5,
      payload: {},
      output: null,
      error: null,
      created_at: new Date().toISOString(),
      dispatched_at: null,
      finished_at: new Date().toISOString(),
      next_attempt_at: new Date().toISOString(),
      expires_at: new Date().toISOString(),
    });
  }),

  // POST /api/v1/admin/operations/:id/retry (202)
  http.post(/\/api\/v1\/admin\/operations\/([^/]+)\/retry$/, ({ params }) => {
    const { 0: id } = params as any;
    const opIndex = mockOperations.findIndex((o) => o.id === id);
    if (opIndex === -1) return problem(404, "not_found", "No such operation");
    const updated = {
      ...mockOperations[opIndex],
      status: "pending" as const,
      retries_count: mockOperations[opIndex].retries_count + 1,
      error: null,
      updated_at: new Date().toISOString(),
    };
    mockOperations[opIndex] = updated;
    return HttpResponse.json(operationView(updated), { status: 202 });
  }),

  // Apply-to-all Operation across cluster nodes (POST /api/v1/admin/cluster/apply-all) TODO(api:T16.06)
  http.post(/\/api\/v1\/admin\/cluster\/apply-all$/, async ({ request }) => {
    let body: any = {};
    try {
      body = await request.json();
    } catch {
      body = {};
    }

    const clusterNodes = [
      { id: "node-master-01", name: "node-master-01 (Primary Controller)" },
      { id: "node-worker-02", name: "node-worker-02 (US-East Worker)" },
      { id: "node-worker-03", name: "node-worker-03 (US-East Worker)" },
      { id: "node-worker-04", name: "node-worker-04 (US-West Worker)" },
      { id: "node-worker-05", name: "node-worker-05 (EU-Central Worker)" },
      { id: "node-worker-06", name: "node-worker-06 (EU-North Worker)" },
      { id: "node-worker-07", name: "node-worker-07 (AP-South Worker)" },
      { id: "node-worker-08", name: "node-worker-08 (AP-East Worker)" },
      { id: "node-worker-09", name: "node-worker-09 (SA-East Worker)" },
      { id: "node-worker-10", name: "node-worker-10 (AF-South Worker)" },
    ];

    const targetNodes = Array.isArray(body.target_node_ids) && body.target_node_ids.length > 0
      ? clusterNodes.filter((n) => body.target_node_ids.includes(n.id))
      : clusterNodes;

    const opType = body.op_type || "SyncSettings";
    const batchSize = Math.max(1, body.batch_size || 2);
    const stopOnFailure = body.stop_on_failure !== false;
    const simulateFailNodeId = body.simulate_fail_node_id || null;

    const totalNodes = targetNodes.length;
    const batchesCount = Math.ceil(totalNodes / batchSize);
    const nodeResults: any[] = [];

    let circuitBroken = false;
    let succeededCount = 0;
    let failedCount = 0;
    let skippedCount = 0;

    for (let i = 0; i < totalNodes; i++) {
      const node = targetNodes[i];
      const batchIndex = Math.floor(i / batchSize);

      if (circuitBroken) {
        nodeResults.push({
          node_id: node.id,
          node_name: node.name,
          batch_index: batchIndex,
          status: "skipped",
          duration_ms: 0,
          error: null,
          output: "Skipped due to circuit breaker trip on earlier node failure",
        });
        skippedCount++;
        continue;
      }

      const isFail = simulateFailNodeId && (
        simulateFailNodeId === node.id ||
        simulateFailNodeId === `node-${String(i + 1).padStart(2, "0")}` ||
        node.id.includes(simulateFailNodeId)
      );

      if (isFail) {
        nodeResults.push({
          node_id: node.id,
          node_name: node.name,
          batch_index: batchIndex,
          status: "failed",
          duration_ms: 120 + i * 15,
          error: `Node agent connection timeout: health check failed on ${node.id}`,
          output: null,
        });
        failedCount++;

        // Add to mockOperations ledger as failed
        mockOperations.unshift({
          id: `op_roll_${Date.now()}_${node.id}`,
          idempotency_key: `idem_roll_${opType}_${node.id}_${Date.now()}`,
          op_type: opType,
          node_id: node.id,
          node_name: node.name,
          tenant_id: null,
          status: "failed",
          retries_count: 0,
          payload: { batch: batchIndex + 1, rolling: true },
          output: null,
          error: {
            code: "ERR_NODE_UNREACHABLE",
            message: `Connection timeout on node ${node.id}`,
          },
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        });

        if (stopOnFailure) {
          circuitBroken = true;
        }
      } else {
        nodeResults.push({
          node_id: node.id,
          node_name: node.name,
          batch_index: batchIndex,
          status: "succeeded",
          duration_ms: 45 + i * 8,
          error: null,
          output: `${opType} executed successfully across node subsystems (exit code 0)`,
        });
        succeededCount++;

        // Add to mockOperations ledger as succeeded
        mockOperations.unshift({
          id: `op_roll_${Date.now()}_${node.id}`,
          idempotency_key: `idem_roll_${opType}_${node.id}_${Date.now()}`,
          op_type: opType,
          node_id: node.id,
          node_name: node.name,
          tenant_id: null,
          status: "succeeded",
          retries_count: 0,
          payload: { batch: batchIndex + 1, rolling: true },
          output: { success: true, exit_code: 0 },
          error: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        });
      }
    }

    const overallStatus = circuitBroken ? "halted" : (failedCount > 0 ? "failed" : "succeeded");
    const executionId = `roll_${Date.now()}`;

    return HttpResponse.json({
      id: executionId,
      op_type: opType,
      total_nodes: totalNodes,
      completed_nodes: succeededCount + failedCount,
      succeeded_nodes: succeededCount,
      failed_nodes: failedCount,
      skipped_nodes: skippedCount,
      status: overallStatus,
      batches_count: batchesCount,
      node_results: nodeResults,
      created_at: new Date().toISOString(),
      completed_at: new Date().toISOString(),
      message: circuitBroken
        ? `Rolling execution halted after node failure in batch. Circuit breaker tripped.`
        : `Rolling execution completed across all ${totalNodes} nodes successfully.`,
    });
  }),

  // Modules: list (GET /api/v1/admin/modules), shaped like `ModuleView` in openapi.json
  http.get(/\/api\/v1\/admin\/modules$/, () => {
    return HttpResponse.json({
      items: modules.map((m) => ({
        name: m.name,
        version: m.version,
        title: m.title,
        category: m.category,
        core: m.core,
        enabled: m.enabled,
        active: m.enabled,
        present: true,
        depends_on: m.depends_on,
        dependents: modules.filter((o) => o.depends_on.includes(m.name)).map((o) => o.name),
      })),
    });
  }),

  // Modules: switch on or off (PUT /api/v1/admin/modules/:name  { enabled, cascade })
  http.put(/\/api\/v1\/admin\/modules\/([^/]+)$/, async ({ params, request }) => {
    const name = params[0] as string;
    const { enabled, cascade = false } = (await request.json().catch(() => ({}))) as { enabled?: boolean; cascade?: boolean };

    const mod = modules.find((m) => m.name === name);
    if (!mod) return problem(404, "not_found", "No such module");
    if (mod.core) return problem(409, "core_module", "A core module cannot be switched off");

    const changed: string[] = [];
    if (!enabled) {
      const dependents = modules.filter((m) => m.enabled && m.depends_on.includes(name));
      if (dependents.length > 0 && !cascade) {
        return problem(409, "dependents_active", `Other modules need it: ${dependents.map((d) => d.name).join(", ")}`);
      }
      for (const dep of dependents) {
        dep.enabled = false;
        dep.status = "disabled";
        changed.push(dep.name);
      }
    }
    mod.enabled = !!enabled;
    mod.status = enabled ? "active" : "disabled";
    changed.push(name);
    return HttpResponse.json({ changed, active: modules.filter((m) => m.enabled).map((m) => m.name) });
  }),

  // Settings: Get all modules and their schemas/values (GET /api/v1/admin/settings)
  // TODO(api:T03.09): Replace with real backend settings endpoint
  http.get(/\/api\/v1\/admin\/settings$/, () => {
    return HttpResponse.json({
      modules: Object.keys(mockSettingsStore).map((key) => ({
        id: key,
        title: mockSettingsStore[key].title,
        description: mockSettingsStore[key].description,
        schema: mockSettingsStore[key].schema,
        values: mockSettingsStore[key].values,
      })),
    });
  }),

  // Settings: Get schema & values for a specific module (GET /api/v1/admin/settings/:module)
  // TODO(api:T03.09): Replace with real backend settings endpoint
  http.get(/\/api\/v1\/admin\/settings\/([^/]+)$/, ({ params }) => {
    const mod = params[0] as string;
    const entry = mockSettingsStore[mod];
    if (!entry) {
      return HttpResponse.json(
        { error: "not_found", message: `Settings module '${mod}' not found` },
        { status: 404 }
      );
    }
    return HttpResponse.json({
      module: mod,
      title: entry.title,
      description: entry.description,
      schema: entry.schema,
      values: entry.values,
    });
  }),

  // Settings: Update values for a module (PUT /api/v1/admin/settings/:module)
  // TODO(api:T03.09): Replace with real backend settings endpoint
  // TODO(api:T03.08): JSON Schema validation
  http.put(/\/api\/v1\/admin\/settings\/([^/]+)$/, async ({ params, request }) => {
    const mod = params[0] as string;
    const entry = mockSettingsStore[mod];
    if (!entry) {
      return HttpResponse.json(
        { error: "not_found", message: `Settings module '${mod}' not found` },
        { status: 404 }
      );
    }

    const body = (await request.json().catch(() => ({}))) as any;
    const incomingValues = body?.values || body;

    // Validate required fields
    const required = entry.schema.required || [];
    for (const reqKey of required) {
      if (
        incomingValues[reqKey] === undefined ||
        incomingValues[reqKey] === null ||
        incomingValues[reqKey] === ""
      ) {
        return HttpResponse.json(
          {
            error: "validation_failed",
            message: `Missing required field: ${reqKey}`,
            field: reqKey,
          },
          { status: 422 }
        );
      }
    }

    // Update store
    mockSettingsStore[mod].values = {
      ...mockSettingsStore[mod].values,
      ...incomingValues,
    };

    return HttpResponse.json({
      success: true,
      module: mod,
      values: mockSettingsStore[mod].values,
    });
  }),

  // Feature Sets: List all feature sets (GET /api/v1/admin/feature-sets)
  // TODO(api:T04.02): Replace with real backend feature sets endpoint
  http.get(/\/api\/v1\/admin\/feature-sets$/, () => {
    return HttpResponse.json({
      items: featureSets,
      total: featureSets.length,
    });
  }),

  // Feature Sets: Create feature set (POST /api/v1/admin/feature-sets)
  // TODO(api:T04.02): Replace with real backend feature sets endpoint
  http.post(/\/api\/v1\/admin\/feature-sets$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const { name, description = "", features = [], is_default = false } = body || {};

    if (!name || typeof name !== "string" || !name.trim()) {
      return HttpResponse.json(
        { error: "bad_request", message: "Feature set name is required" },
        { status: 400 }
      );
    }

    if (is_default) {
      featureSets.forEach((fs) => {
        fs.is_default = false;
      });
    }

    const newFs: MockFeatureSet = {
      id: `fs_${Date.now()}`,
      name: name.trim(),
      description: description.trim(),
      features: Array.isArray(features) ? features : [],
      is_default: Boolean(is_default),
      plans_count: 0,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };

    featureSets.unshift(newFs);
    return HttpResponse.json(newFs, { status: 201 });
  }),

  // Feature Sets: Update feature set (PUT /api/v1/admin/feature-sets/:id)
  // TODO(api:T04.02): Replace with real backend feature sets endpoint
  http.put(/\/api\/v1\/admin\/feature-sets\/([^/]+)$/, async ({ params, request }) => {
    const id = params[0] as string;
    const body = (await request.json().catch(() => ({}))) as any;
    const { name, description, features, is_default } = body || {};

    const index = featureSets.findIndex((fs) => fs.id === id);
    if (index === -1) {
      return HttpResponse.json(
        { error: "not_found", message: "Feature set not found" },
        { status: 404 }
      );
    }

    if (name !== undefined && (!name || !name.trim())) {
      return HttpResponse.json(
        { error: "bad_request", message: "Feature set name cannot be empty" },
        { status: 400 }
      );
    }

    if (is_default) {
      featureSets.forEach((fs) => {
        fs.is_default = false;
      });
    }

    const current = featureSets[index];
    const updated: MockFeatureSet = {
      ...current,
      name: name !== undefined ? name.trim() : current.name,
      description: description !== undefined ? description.trim() : current.description,
      features: Array.isArray(features) ? features : current.features,
      is_default: is_default !== undefined ? Boolean(is_default) : current.is_default,
      updated_at: new Date().toISOString(),
    };

    featureSets[index] = updated;
    return HttpResponse.json(updated);
  }),

  // Feature Sets: Delete feature set (DELETE /api/v1/admin/feature-sets/:id)
  // TODO(api:T04.02): Replace with real backend feature sets endpoint
  http.delete(/\/api\/v1\/admin\/feature-sets\/([^/]+)$/, ({ params }) => {
    const id = params[0] as string;
    const index = featureSets.findIndex((fs) => fs.id === id);
    if (index === -1) {
      return HttpResponse.json(
        { error: "not_found", message: "Feature set not found" },
        { status: 404 }
      );
    }

    if (featureSets[index].is_default) {
      return HttpResponse.json(
        { error: "cannot_delete_default", message: "Cannot delete the default feature set" },
        { status: 400 }
      );
    }

    featureSets.splice(index, 1);
    return HttpResponse.json({ success: true });
  }),

  // Plans: List all hosting plans (GET /api/v1/admin/plans)
  // TODO(api:T04.03): Replace with real backend plan endpoint
  http.get(/\/api\/v1\/admin\/plans$/, () => {
    return HttpResponse.json({
      items: plans,
      total: plans.length,
    });
  }),

  // Plans: Create hosting plan (POST /api/v1/admin/plans)
  // TODO(api:T04.03): Replace with real backend plan endpoint
  http.post(/\/api\/v1\/admin\/plans$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const {
      name,
      description = "",
      feature_set_id = "fs_default",
      feature_set_name = "Default (Full Stack)",
      is_default = false,
      resources = {},
      limits = {},
      node_group_id = null,
      node_group_name = null,
      target_tags = [],
      target_region = null,
    } = body || {};

    if (!name || typeof name !== "string" || !name.trim()) {
      return HttpResponse.json(
        { error: "bad_request", message: "Plan name is required" },
        { status: 400 }
      );
    }

    if (is_default) {
      plans.forEach((p) => {
        p.is_default = false;
      });
    }

    const newPlan: MockPlan = {
      id: `plan_${Date.now()}`,
      name: name.trim(),
      description: description.trim(),
      feature_set_id,
      feature_set_name,
      is_default: Boolean(is_default),
      users_count: 0,
      node_group_id: node_group_id || "ng_global",
      node_group_name: node_group_name || "Global Cluster (Default)",
      target_tags: target_tags || ["general", "standard"],
      target_region: target_region || "global",
      resources: {
        cpu: resources.cpu ?? 1,
        memory_mb: resources.memory_mb ?? 1024,
        disk_mb: resources.disk_mb ?? 10240,
        bandwidth_mb: resources.bandwidth_mb ?? -1,
        pids_limit: resources.pids_limit ?? 100,
      },
      limits: {
        max_domains: limits.max_domains ?? 1,
        max_subdomains: limits.max_subdomains ?? 5,
        max_databases: limits.max_databases ?? 1,
        max_ftp_accounts: limits.max_ftp_accounts ?? 2,
        max_email_accounts: limits.max_email_accounts ?? 5,
      },
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };

    plans.unshift(newPlan);
    return HttpResponse.json(newPlan, { status: 201 });
  }),

  // Plans: Update hosting plan (PUT /api/v1/admin/plans/:id)
  // TODO(api:T04.03): Replace with real backend plan endpoint
  http.put(/\/api\/v1\/admin\/plans\/([^/]+)$/, async ({ params, request }) => {
    const id = (params.id || params[0]) as string;
    const body = (await request.json().catch(() => ({}))) as any;
    const {
      name,
      description,
      feature_set_id,
      feature_set_name,
      is_default,
      resources,
      limits,
      node_group_id,
      node_group_name,
      target_tags,
      target_region,
    } = body || {};

    const index = plans.findIndex((p) => p.id === id);
    if (index === -1) {
      return HttpResponse.json(
        { error: "not_found", message: "Hosting plan not found" },
        { status: 404 }
      );
    }

    if (name !== undefined && (!name || !name.trim())) {
      return HttpResponse.json(
        { error: "bad_request", message: "Plan name cannot be empty" },
        { status: 400 }
      );
    }

    if (is_default) {
      plans.forEach((p) => {
        p.is_default = false;
      });
    }

    const current = plans[index];
    const updated: MockPlan = {
      ...current,
      name: name !== undefined ? name.trim() : current.name,
      description: description !== undefined ? description.trim() : current.description,
      feature_set_id: feature_set_id !== undefined ? feature_set_id : current.feature_set_id,
      feature_set_name: feature_set_name !== undefined ? feature_set_name : current.feature_set_name,
      is_default: is_default !== undefined ? Boolean(is_default) : current.is_default,
      resources: resources !== undefined ? { ...current.resources, ...resources } : current.resources,
      limits: limits !== undefined ? { ...current.limits, ...limits } : current.limits,
      node_group_id: node_group_id !== undefined ? node_group_id : current.node_group_id,
      node_group_name: node_group_name !== undefined ? node_group_name : current.node_group_name,
      target_tags: target_tags !== undefined ? target_tags : current.target_tags,
      target_region: target_region !== undefined ? target_region : current.target_region,
      updated_at: new Date().toISOString(),
    };

    plans[index] = updated;
    return HttpResponse.json(updated);
  }),

  // Plans: Delete hosting plan (DELETE /api/v1/admin/plans/:id)
  // TODO(api:T04.03): Replace with real backend plan endpoint
  http.delete(/\/api\/v1\/admin\/plans\/([^/]+)$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const index = plans.findIndex((p) => p.id === id);
    if (index === -1) {
      return HttpResponse.json(
        { error: "not_found", message: "Hosting plan not found" },
        { status: 404 }
      );
    }

    if (plans[index].is_default) {
      return HttpResponse.json(
        { error: "cannot_delete_default", message: "Cannot delete the default hosting plan" },
        { status: 400 }
      );
    }

    if (plans[index].users_count > 0) {
      return HttpResponse.json(
        { error: "plan_in_use", message: "Cannot delete plan with active assigned users" },
        { status: 400 }
      );
    }

    plans.splice(index, 1);
    return HttpResponse.json({ success: true });
  }),

  // Users: list (GET /api/v1/admin/users?limit&offset&q&status&plan_id&node_id), shaped like `UserView`
  http.get(/\/api\/v1\/admin\/users(\?.*)?$/, ({ request }) => {
    const url = new URL(request.url);
    const q = (url.searchParams.get("q") || "").trim().toLowerCase();
    const status = url.searchParams.get("status");
    const planId = url.searchParams.get("plan_id");
    const nodeId = url.searchParams.get("node_id");
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "50", 10));
    const offset = Math.max(0, parseInt(url.searchParams.get("offset") || "0", 10));

    let filtered = [...mockUsers];
    if (q) {
      filtered = filtered.filter((u) => u.username.toLowerCase().includes(q) || u.email.toLowerCase().includes(q));
    }
    if (status) filtered = filtered.filter((u) => u.status === (status === "provisioning" ? "pending" : status));
    if (planId) filtered = filtered.filter((u) => u.plan_id === planId);
    if (nodeId) filtered = filtered.filter((u) => u.node_id === nodeId);

    return HttpResponse.json({
      items: filtered.slice(offset, offset + limit).map((u) => ({
        id: u.id,
        username: u.username,
        email: u.email,
        locale: "en",
        account_status: u.status === "suspended" ? "disabled" : "active",
        created_at: u.created_at,
        tenant: {
          id: `ten_${u.id}`,
          username: u.username,
          node_id: u.node_id,
          node_name: u.node_name,
          plan_id: u.plan_id,
          plan_name: u.plan_name,
          status: u.status === "pending" ? "provisioning" : u.status,
          purge_after: null,
          // Shares of the plan, from the name, so the rings of the list look alive.
          usage:
            u.status === "active"
              ? {
                  memory_percent: (u.username.length * 7) % 100,
                  cpu_percent: (u.username.length * 3) % 40,
                  disk_percent: (u.username.length * 11) % 100,
                  inodes_percent: (u.username.length * 5) % 60,
                }
              : { memory_percent: 0, cpu_percent: 0, disk_percent: 0, inodes_percent: 0 },
        },
      })),
      total: filtered.length,
    });
  }),

  // Users: change plan (POST /api/v1/admin/users/:id/plan)
  http.post(/\/api\/v1\/admin\/users\/([^/]+)\/plan$/, async ({ params, request }) => {
    const body = (await request.json().catch(() => ({}))) as { plan_id?: string };
    const user = mockUsers.find((u) => u.id === params[0]);
    const plan = mockPlansList.find((p) => p.id === body.plan_id);
    if (!user || !plan) return problem(404, "not_found", "No such user or plan");
    user.plan_id = plan.id;
    user.plan_name = plan.name;
    user.disk_limit_bytes = plan.disk_limit;
    return HttpResponse.json({ ok: true });
  }),

  // Users: Single suspend (POST /api/v1/admin/users/:id/suspend)
  // TODO(api:T04.12): Replace with real backend endpoint
  http.post(/\/api\/v1\/admin\/users\/([^/]+)\/suspend$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const user = mockUsers.find((u) => u.id === id);
    if (!user && !mockDetailStore[id]) {
      return HttpResponse.json({ error: "not_found" }, { status: 404 });
    }
    const suspendedAt = new Date().toISOString();
    if (user) {
      user.status = "suspended";
      user.suspended_at = suspendedAt;
    }
    if (mockDetailStore[id]) {
      mockDetailStore[id].status = "suspended";
      mockDetailStore[id].suspended_at = suspendedAt;
    }
    return HttpResponse.json(user || mockDetailStore[id]);
  }),

  // Users: Single unsuspend (POST /api/v1/admin/users/:id/unsuspend)
  // TODO(api:T04.12): Replace with real backend endpoint
  http.post(/\/api\/v1\/admin\/users\/([^/]+)\/unsuspend$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const user = mockUsers.find((u) => u.id === id);
    if (!user && !mockDetailStore[id]) {
      return HttpResponse.json({ error: "not_found" }, { status: 404 });
    }
    if (user) {
      user.status = "active";
      user.suspended_at = null;
    }
    if (mockDetailStore[id]) {
      mockDetailStore[id].status = "active";
      mockDetailStore[id].suspended_at = null;
    }
    return HttpResponse.json(user || mockDetailStore[id]);
  }),

  // Users: Impersonate user (POST /api/v1/admin/users/:id/impersonate)
  // TODO(api:T04.13): Replace with real impersonation endpoint
  http.post(/\/api\/v1\/admin\/users\/([^/]+)\/impersonate$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const user = mockUsers.find((u) => u.id === id);
    if (!user) {
      return HttpResponse.json({ error: "not_found" }, { status: 404 });
    }
    return HttpResponse.json({
      token: `mock_impersonate_${user.id}_${Date.now()}`,
      expires_in_seconds: 60,
      redirect_url: `/client?impersonate=${user.id}`,
    });
  }),

  // Users: Delete single user (DELETE /api/v1/admin/users/:id)
  // TODO(api:T04.12): Replace with real backend delete endpoint
  http.delete(/\/api\/v1\/admin\/users\/([^/]+)$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const index = mockUsers.findIndex((u) => u.id === id);
    if (index === -1) {
      return HttpResponse.json({ error: "not_found" }, { status: 404 });
    }
    mockUsers.splice(index, 1);
    return new HttpResponse(null, { status: 204 });
  }),

  // Users: Create tenant user with operation & container provisioning (POST /api/v1/admin/users)
  // TODO(api:T04.12): Replace with real backend tenant creation endpoint
  http.post(/\/api\/v1\/admin\/users$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const { username, email, plan_id = "plan_standard", node_id = "node-sgp-01", primary_domain } = body || {};

    if (!username || !email) {
      return HttpResponse.json(
        { title: "Validation Error", detail: "Username and email are required" },
        { status: 400 }
      );
    }

    // Reject scheduling on draining nodes - TODO(api:T16.02)
    if (node_id === "node-fra-02-draining" || node_id === "node-worker-02" || node_id.includes("draining")) {
      return HttpResponse.json(
        { title: "Placement Error", detail: "Target node is in drain mode and cannot accept new tenants" },
        { status: 422 }
      );
    }

    const opId = `op_prov_${Date.now()}`;
    const containerName = `bp-tenant-${username}`;

    return HttpResponse.json(
      {
        user: {
          id: `usr_${Date.now()}`,
          username,
          email,
          status: "active",
          plan_id,
          node_id,
          primary_domain,
        },
        operation_id: opId,
        container: {
          id: `cnt_${username}_${Date.now()}`,
          name: containerName,
          status: "running",
          image: "builtpanel/stack-php83-nginx:latest",
          ip_address: "172.28.0.12",
          created_at: new Date().toISOString(),
        },
      },
      { status: 201 }
    );
  }),


  // Users: Get detailed tenant user (GET /api/v1/admin/users/:id)
  // TODO(api:T04.12): Replace with real backend tenant detail endpoint
  ...(() => {
    return [
      http.get(/\/api\/v1\/admin\/users\/([^/]+)$/, ({ params }) => {
        const id = (params.id || params[0]) as string;
        if (!mockDetailStore[id]) {
          const username = id.startsWith("usr_") ? "alpha" : id;
          mockDetailStore[id] = {
            id,
            username,
            email: `${username}@example.com`,
            status: "active",
            uid: 1001,
            root_path: `/srv/tenants/${username}`,
            plan_id: "plan_standard",
            plan_name: "Standard Business",
            node_id: "node-sgp-01",
            node_name: "node-sgp-01 (Singapore)",
            primary_domain: `${username}.com`,
            created_at: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
            suspended_at: null,
            suspended_reason: null,
            resources: {
              cpu_cores: 2,
              cpu_used_pct: 28,
              memory_limit_mb: 2048,
              memory_used_mb: 850,
              disk_limit_mb: 25600,
              disk_used_mb: 1240,
              inodes_limit: 250000,
              inodes_used: 12450,
              bandwidth_limit_mb: -1,
              bandwidth_used_mb: 4800,
              pids_limit: 256,
              pids_used: 42,
            },
            container: {
              id: `cnt_${username}_01`,
              name: `bp-tenant-${username}`,
              status: "running",
              image: "builtpanel/stack-php83-nginx:latest",
              ip_address: "172.28.0.5",
              started_at: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
              uptime: "Up 14 days",
            },
            domains: [
              {
                id: "dom_01",
                domain: `${username}.com`,
                is_primary: true,
                docroot: `/srv/tenants/${username}/domains/${username}.com/public_html`,
                ssl_status: "active",
                php_version: "PHP 8.3",
              },
              {
                id: "dom_02",
                domain: `blog.${username}.com`,
                is_primary: false,
                docroot: `/srv/tenants/${username}/domains/blog.${username}.com/public_html`,
                ssl_status: "active",
                php_version: "PHP 8.3",
              },
            ],
            databases: [
              {
                id: "db_01",
                name: `${username}_wp`,
                type: "mysql",
                size_mb: 48,
                charset: "utf8mb4",
              },
              {
                id: "db_02",
                name: `${username}_app`,
                type: "postgres",
                size_mb: 18,
                charset: "UTF8",
              },
            ],
            logs: [
              {
                id: "log_01",
                timestamp: new Date(Date.now() - 60 * 1000 * 2).toISOString(),
                level: "info",
                source: "nginx",
                message: '192.168.1.105 - - "GET / HTTP/2.0" 200 4820 "-" "Mozilla/5.0 Chrome/130.0"',
              },
              {
                id: "log_02",
                timestamp: new Date(Date.now() - 60 * 1000 * 5).toISOString(),
                level: "info",
                source: "php-fpm",
                message: "[pool www] child 42 ready to handle requests",
              },
              {
                id: "log_03",
                timestamp: new Date(Date.now() - 60 * 1000 * 15).toISOString(),
                level: "warn",
                source: "nginx",
                message: 'Client sent invalid header: "X-Forwarded-Host: evil.com", dropped',
              },
            ],
            activities: [
              {
                id: "act_01",
                action: "user.login",
                actor: username,
                timestamp: new Date(Date.now() - 3600 * 1000 * 2).toISOString(),
                status: "success",
                details: "Client portal session initiated from 192.168.1.105",
              },
              {
                id: "act_02",
                action: "tenant.container_start",
                actor: "system",
                timestamp: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
                status: "success",
                details: "Rootless container stack provisioned and healthy",
              },
            ],
          };
        }
        return HttpResponse.json(mockDetailStore[id]);
      }),

      // Suspend
      http.post(/\/api\/v1\/admin\/users\/([^/]+)\/suspend$/, ({ params }) => {
        const id = (params.id || params[0]) as string;
        if (mockDetailStore[id]) {
          mockDetailStore[id].status = "suspended";
          mockDetailStore[id].suspended_at = new Date().toISOString();
        }
        return HttpResponse.json({ success: true, status: "suspended" });
      }),

      // Unsuspend
      http.post(/\/api\/v1\/admin\/users\/([^/]+)\/unsuspend$/, ({ params }) => {
        const id = (params.id || params[0]) as string;
        if (mockDetailStore[id]) {
          mockDetailStore[id].status = "active";
          mockDetailStore[id].suspended_at = null;
        }
        return HttpResponse.json({ success: true, status: "active" });
      }),

      // Password Reset
      http.post(/\/api\/v1\/admin\/users\/([^/]+)\/password$/, () => {
        return HttpResponse.json({ success: true, message: "Password updated successfully" });
      }),

      // Impersonate
      http.post(/\/api\/v1\/admin\/users\/([^/]+)\/impersonate$/, ({ params }) => {
        const id = (params.id || params[0]) as string;
        return HttpResponse.json({
          token: `mock_imp_token_${Date.now()}`,
          expires_in_seconds: 60,
          redirect_url: `/client?user=${id}`,
        });
      }),

      // Delete
      http.delete(/\/api\/v1\/admin\/users\/([^/]+)$/, ({ params }) => {
        const id = (params.id || params[0]) as string;
        delete mockDetailStore[id];
        return new HttpResponse(null, { status: 204 });
      }),
    ];
  })(),

  // Client: Dashboard overview & resource quotas (GET /api/v1/client/dashboard)
  // TODO(api:T04.18): Replace with real backend client dashboard endpoint
  http.get(/\/api\/v1\/client\/dashboard$/, () => {
    return HttpResponse.json({
      tenant: {
        id: "usr_tenant_01",
        username: "client_alpha",
        email: "alpha@example.com",
        primary_domain: "alpha-hosting.com",
        plan_name: "Standard Business Plan",
        node_name: "node-sgp-01 (Singapore)",
        server_ip: "167.104.101.81",
        nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
        created_at: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
        status: "active",
      },
      container: {
        status: "running",
        name: "bp-tenant-alpha",
        image: "builtpanel/stack-php83-nginx:latest",
        php_version: "8.3.12",
        uptime: "Up 14 days, 6 hours",
        ip_address: "172.28.0.5",
      },
      resources: {
        disk_limit_mb: 25600,
        disk_used_mb: 1280,
        bandwidth_limit_mb: -1,
        bandwidth_used_mb: 4800,
        memory_limit_mb: 2048,
        memory_used_mb: 850,
        cpu_cores: 2,
        cpu_used_pct: 28,
        inodes_limit: 250000,
        inodes_used: 12450,
        databases_limit: 10,
        databases_used: 2,
        domains_limit: 5,
        domains_used: 2,
        emails_limit: 10,
        emails_used: 1,
      },
      primary_domain: {
        domain: "alpha-hosting.com",
        docroot: "/srv/tenants/client_alpha/domains/alpha-hosting.com/public_html",
        ssl_status: "active",
        ssl_issuer: "Let's Encrypt Authority X3",
        ssl_expires_in_days: 76,
        php_version: "PHP 8.3",
      },
      shortcuts: [
        { id: "files", title: "File Manager", icon: "Folder", href: "/files", phase: 5 },
        { id: "domains", title: "Domain Names", icon: "Globe", href: "/domains", phase: 6 },
        { id: "databases", title: "Databases", icon: "Database", href: "/databases", phase: 7 },
        { id: "ssl", title: "SSL / TLS", icon: "Lock", href: "/ssl", phase: 6 },
        { id: "cron", title: "Cron Jobs", icon: "Clock", href: "/cron", phase: 5 },
        { id: "php", title: "PHP Configuration", icon: "Code", href: "/php", phase: 6 },
        { id: "backups", title: "Backup & Restore", icon: "Archive", href: "/backups", phase: 8 },
        { id: "security", title: "Account Security", icon: "ShieldCheck", href: "/security", phase: 1 },
      ],
      last_backup: {
        created_at: new Date(Date.now() - 3600 * 1000 * 18).toISOString(),
        size_mb: 248,
        status: "completed",
        type: "automatic",
      },
      ssl_warnings: [
        {
          domain: "staging.alpha-hosting.com",
          issuer: "Let's Encrypt",
          expires_in_days: 7,
          severity: "warning",
        },
      ],
      recent_events: [
        {
          id: "ev_01",
          action: "user.login",
          timestamp: new Date(Date.now() - 600 * 1000).toISOString(),
          ip_address: "192.168.1.105",
          description: "Client portal session active",
        },
        {
          id: "ev_02",
          action: "ssl.renewed",
          timestamp: new Date(Date.now() - 86400 * 1000 * 2).toISOString(),
          ip_address: "system",
          description: "SSL Certificate auto-renewed for alpha-hosting.com",
        },
        {
          id: "ev_03",
          action: "container.start",
          timestamp: new Date(Date.now() - 86400 * 1000 * 14).toISOString(),
          ip_address: "node-sgp-01",
          description: "Container stack started and healthy",
        },
      ],
    });
  }),

  // ---- Client domains, shaped like `DomainView`, `SslView`, `RedirectView` in openapi.json.
  // GET /api/v1/client/domains
  http.get(/\/api\/v1\/client\/domains$/, () => {
    return HttpResponse.json({ items: mockClientDomains.map(clientDomainView), total: mockClientDomains.length });
  }),

  // POST /api/v1/client/domains
  http.post(/\/api\/v1\/client\/domains$/, async ({ request }) => {
    const body = (await request.json()) as any;
    if (!body || !body.fqdn) return problem(422, "required", "The domain name is required");
    const fqdn = String(body.fqdn).trim().toLowerCase();
    if (mockClientDomains.some((d) => d.name.toLowerCase() === fqdn)) {
      return problem(409, "domain_taken", `Domain ${fqdn} is already configured`);
    }
    if (mockClientDomains.length >= 5) return problem(409, "limit_reached", "Domain limit reached for your plan");
    const kind = body.kind === "sub" ? "subdomain" : body.kind || "addon";
    const created: MockClientDomain = {
      id: `dom_${Date.now()}`,
      name: fqdn,
      type: kind,
      docroot: `/${body.docroot ?? ""}`,
      is_primary: false,
      ssl_status: "none",
      ssl_expires_at: null,
      ssl_issuer: null,
      dns_status: "pending",
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date().toISOString(),
    };
    mockClientDomains = [created, ...mockClientDomains];
    return HttpResponse.json(clientDomainView(created), { status: 201 });
  }),

  // GET /api/v1/client/domains/:id
  http.get(/\/api\/v1\/client\/domains\/([^/]+)$/, ({ params }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    return domain ? HttpResponse.json(clientDomainView(domain)) : problem(404, "not_found", "No such domain");
  }),

  // PATCH /api/v1/client/domains/:id (docroot, force_https, hsts)
  http.patch(/\/api\/v1\/client\/domains\/([^/]+)$/, async ({ params, request }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain) return problem(404, "not_found", "No such domain");
    const body = (await request.json()) as { docroot?: string; force_https?: boolean; hsts?: boolean };
    if (body.docroot !== undefined) domain.docroot = `/${body.docroot}`;
    const cert = mockSslCertificates.find((c) => c.domain_id === domain.id);
    if (cert) {
      if (body.force_https !== undefined) cert.force_https = body.force_https;
      if (body.hsts !== undefined) cert.hsts_enabled = body.hsts;
    }
    return HttpResponse.json(clientDomainView(domain));
  }),

  // DELETE /api/v1/client/domains/:id
  http.delete(/\/api\/v1\/client\/domains\/([^/]+)$/, ({ params }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain) return problem(404, "not_found", "No such domain");
    if (domain.is_primary) return problem(409, "primary_domain", "The primary domain cannot be removed");
    mockClientDomains = mockClientDomains.filter((d) => d.id !== domain.id);
    return new HttpResponse(null, { status: 204 });
  }),

  // GET /api/v1/client/domains/:id/redirects, POST, PATCH/DELETE .../:redirect_id
  http.get(/\/api\/v1\/client\/domains\/([^/]+)\/redirects$/, ({ params }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain) return problem(404, "not_found", "No such domain");
    return HttpResponse.json({
      items: domain.redirect
        ? [
            {
              id: `red_${domain.id}`,
              domain_id: domain.id,
              source_path: "/",
              target: domain.redirect.target_url,
              code: domain.redirect.status_code,
              created_at: domain.created_at,
            },
          ]
        : [],
    });
  }),
  http.post(/\/api\/v1\/client\/domains\/([^/]+)\/redirects$/, async ({ params, request }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain) return problem(404, "not_found", "No such domain");
    const body = (await request.json()) as { source_path: string; target: string; code?: number };
    domain.redirect = { enabled: true, target_url: body.target, status_code: body.code === 302 ? 302 : 301 };
    return HttpResponse.json(
      {
        id: `red_${domain.id}`,
        domain_id: domain.id,
        source_path: body.source_path,
        target: body.target,
        code: body.code ?? 301,
        created_at: new Date().toISOString(),
      },
      { status: 201 },
    );
  }),
  http.patch(/\/api\/v1\/client\/domains\/([^/]+)\/redirects\/([^/]+)$/, async ({ params, request }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain?.redirect) return problem(404, "not_found", "No such redirect");
    const body = (await request.json()) as { target?: string; code?: number };
    domain.redirect = {
      enabled: true,
      target_url: body.target ?? domain.redirect.target_url,
      status_code: (body.code ?? domain.redirect.status_code) === 302 ? 302 : 301,
    };
    return HttpResponse.json({
      id: `red_${domain.id}`,
      domain_id: domain.id,
      source_path: "/",
      target: domain.redirect.target_url,
      code: domain.redirect.status_code,
      created_at: domain.created_at,
    });
  }),
  http.delete(/\/api\/v1\/client\/domains\/([^/]+)\/redirects\/([^/]+)$/, ({ params }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain?.redirect) return problem(404, "not_found", "No such redirect");
    domain.redirect = null;
    return new HttpResponse(null, { status: 204 });
  }),

  // Admin: domains of every account (GET /api/v1/admin/domains?limit&offset&q&status&kind), shaped like `DomainView`
  http.get(/\/api\/v1\/admin\/domains$/, ({ request }) => {
    const url = new URL(request.url);
    const q = (url.searchParams.get("q") || "").toLowerCase().trim();
    const status = url.searchParams.get("status");
    const kind = url.searchParams.get("kind");
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "50", 10));
    const offset = Math.max(0, parseInt(url.searchParams.get("offset") || "0", 10));

    const filtered = mockAdminDomains.filter((d) => {
      const matchSearch = !q || d.name.toLowerCase().includes(q) || d.tenant_username.toLowerCase().includes(q);
      const matchStatus = !status || d.status === status;
      const matchKind = !kind || (kind === "sub" ? d.type === "subdomain" : d.type === kind);
      return matchSearch && matchStatus && matchKind;
    });

    return HttpResponse.json({
      items: filtered.slice(offset, offset + limit).map((d) => ({
        id: d.id,
        fqdn: d.name,
        unicode: d.name,
        kind: d.type === "subdomain" ? "sub" : d.type,
        docroot: d.docroot.replace(/^\/+/, ""),
        force_https: false,
        hsts: false,
        status: d.status,
        parent_domain_id: null,
        tenant_id: d.tenant_id,
        tenant_username: d.tenant_username,
        suspended_at: d.suspended_at ?? null,
        created_at: d.created_at,
      })),
      total: filtered.length,
    });
  }),

  // Admin: add a domain to an account (POST /api/v1/admin/domains  { user_id, fqdn })
  http.post(/\/api\/v1\/admin\/domains$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as { user_id?: string; fqdn?: string };
    const name = String(body.fqdn ?? "").toLowerCase();
    if (!name) return problem(422, "required", "The domain name is required");
    if (mockAdminDomains.some((d) => d.name === name)) return problem(409, "domain_taken", `Domain ${name} is already configured`);
    const item: MockAdminDomain = {
      id: `adm_bulk_${Date.now()}_${mockAdminDomains.length}`,
      name,
      type: "addon" as const,
      tenant_id: body.user_id ?? "usr_0001",
      tenant_username: "alpha",
      node_id: "node-sgp-01",
      node_name: "node-sgp-01 (Cluster)",
      docroot: `/domains/${name}/public_html`,
      is_primary: false,
      status: "active" as const,
      ssl_status: "none" as const,
      dns_status: "pending" as const,
      redirect: null,
      php_version: "PHP 8.3",
      created_at: new Date().toISOString(),
    };
    // New ones go to the top, in the order they were added.
    const at = mockAdminDomains.filter((d) => d.id.startsWith("adm_bulk_")).length;
    mockAdminDomains = [...mockAdminDomains.slice(0, at), item, ...mockAdminDomains.slice(at)];
    return HttpResponse.json({ id: item.id, fqdn: item.name }, { status: 201 });
  }),

  // Admin: Suspend domain (POST /api/v1/admin/domains/:id/suspend)
  http.post(/\/api\/v1\/admin\/domains\/([^/]+)\/suspend$/, ({ params }) => {
    const id = params[0] as string;
    const domain = mockAdminDomains.find((d) => d.id === id);
    if (!domain) {
      return HttpResponse.json({ message: "Domain not found" }, { status: 404 });
    }
    domain.status = "suspended";
    domain.suspended_at = new Date().toISOString();
    return HttpResponse.json(domain);
  }),

  // Admin: Unsuspend domain (POST /api/v1/admin/domains/:id/unsuspend)
  http.post(/\/api\/v1\/admin\/domains\/([^/]+)\/unsuspend$/, ({ params }) => {
    const id = params[0] as string;
    const domain = mockAdminDomains.find((d) => d.id === id);
    if (!domain) {
      return HttpResponse.json({ message: "Domain not found" }, { status: 404 });
    }
    domain.status = "active";
    domain.suspended_at = null;
    return HttpResponse.json(domain);
  }),

  // Admin: Delete domain (DELETE /api/v1/admin/domains/:id)
  http.delete(/\/api\/v1\/admin\/domains\/([^/]+)$/, ({ params }) => {
    const id = params[0] as string;
    const domain = mockAdminDomains.find((d) => d.id === id);
    if (!domain) {
      return HttpResponse.json({ message: "Domain not found" }, { status: 404 });
    }
    mockAdminDomains = mockAdminDomains.filter((d) => d.id !== id);
    return new HttpResponse(null, { status: 204 });
  }),

  // GET /api/v1/client/dns/nameservers
  http.get(/\/api\/v1\/client\/dns\/nameservers$/, () => {
    return HttpResponse.json({ nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"] });
  }),

  // A domain's zone: records are listed, added, changed and removed per domain, shaped like `RecordView`.
  // GET /api/v1/client/domains/:id/dns/records
  http.get(/\/api\/v1\/client\/domains\/([^/]+)\/dns\/records$/, ({ params }) => {
    const zone = mockDnsZones.find((z) => z.domain_id === params[0]);
    if (!zone) return problem(404, "not_found", "No such domain");
    const domain = zone.domain_name;
    return HttpResponse.json({
      zone: { name: domain, status: "active", last_error: null },
      items: mockDnsRecords
        .filter((r) => r.zone_id === zone.id)
        .map((r) => ({
          id: r.id,
          name: r.name,
          fqdn: r.name === "@" ? domain : `${r.name}.${domain}`,
          type: r.type,
          content:
            r.type === "SRV"
              ? `${r.weight ?? 0} ${r.port ?? 0} ${r.content}`
              : r.type === "CAA"
                ? `${r.flags ?? 0} ${r.tag ?? "issue"} "${r.content}"`
                : r.content,
          ttl: r.ttl,
          priority: r.priority ?? null,
        })),
    });
  }),

  // POST /api/v1/client/domains/:id/dns/records
  http.post(/\/api\/v1\/client\/domains\/([^/]+)\/dns\/records$/, async ({ params, request }) => {
    const zone = mockDnsZones.find((z) => z.domain_id === params[0]);
    if (!zone) return problem(404, "not_found", "No such domain");
    const body = (await request.json()) as { name: string; type: string; content: string; ttl?: number; priority?: number };
    const name = body.name || "@";
    if (body.type === "CNAME" && mockDnsRecords.some((r) => r.zone_id === zone.id && r.name === name)) {
      return problem(409, "cname_conflict", "A CNAME cannot share its name with another record");
    }
    if (body.type !== "CNAME" && mockDnsRecords.some((r) => r.zone_id === zone.id && r.name === name && r.type === "CNAME")) {
      return problem(409, "cname_conflict", "That name is a CNAME");
    }
    if ((body.type === "MX" || body.type === "SRV") && body.priority === undefined) {
      return problem(422, "priority_required", "A priority is required for this type");
    }
    const created: MockDnsRecord = {
      id: `rec_${Date.now()}`,
      zone_id: zone.id,
      name,
      type: body.type,
      content: body.content,
      ttl: body.ttl ?? 3600,
      priority: body.priority ?? null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    mockDnsRecords = [created, ...mockDnsRecords];
    zone.records_count = mockDnsRecords.filter((r) => r.zone_id === zone.id).length;
    return HttpResponse.json(
      {
        id: created.id,
        name,
        fqdn: name === "@" ? zone.domain_name : `${name}.${zone.domain_name}`,
        type: created.type,
        content: created.content,
        ttl: created.ttl,
        priority: created.priority,
      },
      { status: 201 },
    );
  }),

  // PATCH /api/v1/client/domains/:id/dns/records/:record_id (data, TTL, priority)
  http.patch(/\/api\/v1\/client\/domains\/([^/]+)\/dns\/records\/([^/]+)$/, async ({ params, request }) => {
    const record = mockDnsRecords.find((r) => r.id === params[1]);
    if (!record) return problem(404, "not_found", "No such record");
    const body = (await request.json()) as { content?: string; ttl?: number; priority?: number };
    if (body.content !== undefined) record.content = body.content;
    if (body.ttl !== undefined) record.ttl = body.ttl;
    if (body.priority !== undefined) record.priority = body.priority;
    const zone = mockDnsZones.find((z) => z.id === record.zone_id);
    return HttpResponse.json({
      id: record.id,
      name: record.name,
      fqdn: record.name === "@" ? zone?.domain_name : `${record.name}.${zone?.domain_name}`,
      type: record.type,
      content: record.content,
      ttl: record.ttl,
      priority: record.priority ?? null,
    });
  }),

  // DELETE /api/v1/client/domains/:id/dns/records/:record_id
  http.delete(/\/api\/v1\/client\/domains\/([^/]+)\/dns\/records\/([^/]+)$/, ({ params }) => {
    const record = mockDnsRecords.find((r) => r.id === params[1]);
    if (!record) return problem(404, "not_found", "No such record");
    mockDnsRecords = mockDnsRecords.filter((r) => r.id !== record.id);
    const zone = mockDnsZones.find((z) => z.id === record.zone_id);
    if (zone) zone.records_count = mockDnsRecords.filter((r) => r.zone_id === zone.id).length;
    return new HttpResponse(null, { status: 204 });
  }),

  // POST /api/v1/client/domains/:id/dns/check
  http.post(/\/api\/v1\/client\/domains\/([^/]+)\/dns\/check$/, ({ params }) => {
    const zone = mockDnsZones.find((z) => z.domain_id === params[0]);
    if (!zone) return problem(404, "not_found", "No such domain");
    return HttpResponse.json({
      cached: false,
      checked_at: new Date().toISOString(),
      status: "ok",
      expected: zone.nameservers,
      found: zone.nameservers,
      missing: [],
      unexpected: [],
    });
  }),

  // DNS Cluster: List cluster nameservers (GET /api/v1/admin/cluster/dns/servers)
  // TODO(api:T16.04): Replace with real backend DNS cluster endpoint
  http.get(/\/api\/v1\/admin\/cluster\/dns\/servers$/, () => {
    const total = mockDnsClusterServers.length;
    const online = mockDnsClusterServers.filter((s) => s.status === "online").length;
    const primary = mockDnsClusterServers.filter((s) => s.role === "primary").length;
    const secondary = mockDnsClusterServers.filter((s) => s.role !== "primary").length;

    return HttpResponse.json({
      items: mockDnsClusterServers,
      overview: {
        total_servers: total,
        online_servers: online,
        primary_count: primary,
        secondary_count: secondary,
        total_zones: 14,
        cluster_healthy: online >= 2,
        replication_lag_ms: 18,
      },
    });
  }),

  // DNS Cluster: Add new DNS server (POST /api/v1/admin/cluster/dns/servers)
  // TODO(api:T16.04): Replace with real backend DNS cluster endpoint
  http.post(/\/api\/v1\/admin\/cluster\/dns\/servers$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const {
      name,
      hostname,
      ip_address,
      port = 53,
      api_url,
      role = "secondary",
      sync_strategy = "native_db",
    } = body || {};

    if (!name || !hostname || !ip_address) {
      return HttpResponse.json(
        { message: "Name, hostname and IP address are required" },
        { status: 400 }
      );
    }

    const newServer: MockDnsClusterServer = {
      id: `dns_srv_${Date.now()}`,
      name: name.trim(),
      hostname: hostname.trim().toLowerCase(),
      ip_address: ip_address.trim(),
      port: Number(port) || 53,
      api_url: api_url || `http://${ip_address.trim()}:8081`,
      role,
      status: "online",
      sync_strategy,
      zones_count: 14,
      last_sync_at: new Date().toISOString(),
      latency_ms: Math.floor(Math.random() * 40) + 10,
      version: "PowerDNS Authoritative 4.9.1",
    };

    mockDnsClusterServers.push(newServer);
    return HttpResponse.json(newServer, { status: 201 });
  }),

  // DNS Cluster: Update DNS server (PUT /api/v1/admin/cluster/dns/servers/:id)
  // TODO(api:T16.04): Replace with real backend DNS cluster endpoint
  http.put(/\/api\/v1\/admin\/cluster\/dns\/servers\/([^/]+)$/, async ({ params, request }) => {
    const id = (params.id || params[0]) as string;
    const body = (await request.json().catch(() => ({}))) as any;

    const index = mockDnsClusterServers.findIndex((s) => s.id === id);
    if (index === -1) {
      return HttpResponse.json({ message: "DNS server not found" }, { status: 404 });
    }

    const current = mockDnsClusterServers[index];
    const updated: MockDnsClusterServer = {
      ...current,
      ...body,
      id: current.id,
    };

    mockDnsClusterServers[index] = updated;
    return HttpResponse.json(updated);
  }),

  // DNS Cluster: Delete DNS server (DELETE /api/v1/admin/cluster/dns/servers/:id)
  // TODO(api:T16.04): Replace with real backend DNS cluster endpoint
  http.delete(/\/api\/v1\/admin\/cluster\/dns\/servers\/([^/]+)$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const index = mockDnsClusterServers.findIndex((s) => s.id === id);
    if (index === -1) {
      return HttpResponse.json({ message: "DNS server not found" }, { status: 404 });
    }

    if (mockDnsClusterServers[index].role === "primary") {
      const remainingPrimaries = mockDnsClusterServers.filter(
        (s) => s.role === "primary" && s.id !== id
      );
      if (remainingPrimaries.length === 0) {
        return HttpResponse.json(
          { message: "Cannot delete the only primary DNS server in cluster" },
          { status: 400 }
        );
      }
    }

    mockDnsClusterServers.splice(index, 1);
    return HttpResponse.json({ success: true });
  }),

  // DNS Cluster: Manual Sync server (POST /api/v1/admin/cluster/dns/servers/:id/sync)
  // TODO(api:T16.04): Replace with real backend DNS cluster endpoint
  http.post(/\/api\/v1\/admin\/cluster\/dns\/servers\/([^/]+)\/sync$/, ({ params }) => {
    const id = (params.id || params[0]) as string;
    const server = mockDnsClusterServers.find((s) => s.id === id);
    if (!server) {
      return HttpResponse.json({ message: "DNS server not found" }, { status: 404 });
    }

    server.last_sync_at = new Date().toISOString();
    return HttpResponse.json({ success: true, synced_zones: server.zones_count });
  }),

  // DNS Cluster: High-Availability Resolution Test (POST /api/v1/admin/cluster/dns/resolve-test)
  // TODO(api:T16.04): Replace with real backend DNS resolution endpoint
  http.post(/\/api\/v1\/admin\/cluster\/dns\/resolve-test$/, async ({ request }) => {
    const body = (await request.json().catch(() => ({}))) as any;
    const { domain = "example.com", record_type = "A", simulate_offline_server_id } = body || {};

    const survivingServers = mockDnsClusterServers.filter(
      (s) => s.status === "online" && s.id !== simulate_offline_server_id
    );

    const offlineDetected = mockDnsClusterServers
      .filter((s) => s.status === "offline" || s.id === simulate_offline_server_id)
      .map((s) => `${s.name} (${s.hostname})`);

    if (survivingServers.length === 0) {
      return HttpResponse.json({
        domain,
        record_type,
        resolved: false,
        answering_server: "None",
        answering_ip: "N/A",
        offline_servers_detected: offlineDetected,
        redundancy_active: false,
        records: [],
        resolution_time_ms: 0,
        message: "All DNS servers in the cluster are offline. Resolution failed.",
      });
    }

    // Surviving node responds! (DoD T16.04: "একটা DNS server বন্ধ থাকলেও resolve হয়")
    const answering = survivingServers[0];
    const isFailover = offlineDetected.length > 0;

    return HttpResponse.json({
      domain,
      record_type,
      resolved: true,
      answering_server: answering.name,
      answering_ip: answering.ip_address,
      offline_servers_detected: offlineDetected,
      redundancy_active: isFailover,
      records: record_type === "AAAA" ? ["2606:2800:220:1:248:1893:25c8:1946"] : ["93.184.216.34"],
      resolution_time_ms: Math.floor(Math.random() * 15) + 5,
      message: isFailover
        ? `DoD T16.04 Verified: Server ${offlineDetected.join(", ")} was unreachable, but query successfully failed over and resolved via surviving cluster node ${answering.hostname}.`
        : `Query resolved successfully through cluster primary ${answering.hostname}.`,
    });
  }),

  // GET /api/v1/client/domains/:id/ssl
  http.get(/\/api\/v1\/client\/domains\/([^/]+)\/ssl$/, ({ params }) => {
    const view = sslViewOf(params[0] as string);
    return view ? HttpResponse.json(view) : problem(404, "not_found", "No such domain");
  }),

  // PUT /api/v1/client/domains/:id/ssl/custom  { certificate, private_key }
  http.put(/\/api\/v1\/client\/domains\/([^/]+)\/ssl\/custom$/, async ({ params, request }) => {
    const domain = mockClientDomains.find((d) => d.id === params[0]);
    if (!domain) return problem(404, "not_found", "No such domain");
    const body = (await request.json()) as { certificate: string; private_key: string };
    if (!body.certificate?.includes("BEGIN CERTIFICATE")) {
      return problem(422, "invalid_certificate", "That is not a PEM certificate");
    }
    let cert = mockSslCertificates.find((c) => c.domain_id === domain.id);
    if (!cert) {
      cert = {
        id: `cert_${Date.now()}`,
        domain_id: domain.id,
        domain_name: domain.name,
        status: "active",
        type: "custom",
        issuer: null,
        subject: `CN=${domain.name}`,
        sans: [domain.name],
        expires_at: null,
        days_remaining: 0,
        auto_renew: false,
        force_https: false,
        hsts_enabled: false,
        hsts_subdomains: false,
        hsts_preload: false,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      mockSslCertificates.push(cert);
    }
    cert.status = "active";
    cert.type = "custom";
    cert.issuer = "Custom CA";
    cert.expires_at = new Date(Date.now() + 365 * 86400 * 1000).toISOString();
    cert.days_remaining = 365;
    cert.updated_at = new Date().toISOString();
    return HttpResponse.json(sslViewOf(domain.id));
  }),

  // DELETE /api/v1/client/domains/:id/ssl/custom: back to the web server's own certificate
  http.delete(/\/api\/v1\/client\/domains\/([^/]+)\/ssl\/custom$/, ({ params }) => {
    const cert = mockSslCertificates.find((c) => c.domain_id === params[0] && c.type === "custom");
    if (!cert) return problem(404, "not_found", "No uploaded certificate");
    cert.type = "lets_encrypt";
    cert.issuer = "Let's Encrypt";
    return new HttpResponse(null, { status: 204 });
  }),

  // Web server templates, shaped like `TemplateView` and the version list in openapi.json
  ...(() => {
    const view = (t: (typeof mockVhostTemplates)[number]) => ({
      id: t.id,
      name: t.name,
      server: t.server_type,
      body: t.content,
      version: t.version,
      is_default: t.is_default,
      updated_at: t.updated_at,
    });
    return [
      http.get(/\/api\/v1\/admin\/vhost-templates$/, () =>
        HttpResponse.json({
          items: mockVhostTemplates.map((t) => ({ id: t.id, name: t.name, server: t.server_type, version: t.version, is_default: t.is_default, updated_at: t.updated_at })),
        }),
      ),

      // POST /api/v1/admin/vhost-templates/apply (before the :id routes)
      http.post(/\/api\/v1\/admin\/vhost-templates\/apply$/, () => HttpResponse.json({ tenants: 4 }, { status: 202 })),

      // POST /api/v1/admin/vhost-templates/preview  { body }
      http.post(/\/api\/v1\/admin\/vhost-templates\/preview$/, async ({ request }) => {
        const body = (await request.json()) as { body: string };
        return HttpResponse.json({ rendered: body.body.replace(/\{\{\s*domain\s*\}\}/g, "my-awesome-site.com") });
      }),

      http.get(/\/api\/v1\/admin\/vhost-templates\/([^/]+)$/, ({ params }) => {
        const tpl = mockVhostTemplates.find((t) => t.id === params[0]);
        return tpl ? HttpResponse.json(view(tpl)) : problem(404, "not_found", "No such template");
      }),

      // PUT /api/v1/admin/vhost-templates/:id  { name, body }: the next version
      http.put(/\/api\/v1\/admin\/vhost-templates\/([^/]+)$/, async ({ params, request }) => {
        const tpl = mockVhostTemplates.find((t) => t.id === params[0]);
        if (!tpl) return problem(404, "not_found", "No such template");
        const body = (await request.json()) as { name?: string; body: string };
        tpl.content = body.body;
        if (body.name) tpl.name = body.name;
        tpl.version += 1;
        tpl.updated_at = new Date().toISOString();
        mockVhostVersions.unshift({
          id: `ver_${tpl.id}_${tpl.version}`,
          template_id: tpl.id,
          version: tpl.version,
          content: tpl.content,
          changelog: tpl.name,
          created_by: "admin",
          created_at: tpl.updated_at,
        });
        return HttpResponse.json(view(tpl));
      }),

      // GET /api/v1/admin/vhost-templates/:id/versions
      http.get(/\/api\/v1\/admin\/vhost-templates\/([^/]+)\/versions$/, ({ params }) =>
        HttpResponse.json({
          items: mockVhostVersions
            .filter((v) => v.template_id === params[0])
            .map((v) => ({ version: v.version, name: v.changelog, body: v.content, changed_by: v.created_by, created_at: v.created_at })),
        }),
      ),

      // POST /api/v1/admin/vhost-templates/:id/default
      http.post(/\/api\/v1\/admin\/vhost-templates\/([^/]+)\/default$/, ({ params }) => {
        mockVhostTemplates.forEach((t) => {
          t.is_default = t.id === params[0];
        });
        const tpl = mockVhostTemplates.find((t) => t.id === params[0]);
        return tpl ? HttpResponse.json(view(tpl)) : problem(404, "not_found", "No such template");
      }),
    ];
  })(),

  // GET /api/v1/client/php: the account's version and its domains', shaped like `PhpInfo` in openapi.json
  http.get(/\/api\/v1\/client\/php$/, () => {
    return HttpResponse.json({
      account_version: mockClientPhpConfig.default_version,
      applied: true,
      available: mockClientPhpConfig.supported_versions.map((v) => v.version),
      domains: mockClientPhpConfig.domain_configs.map((d) => ({
        id: d.domain_id,
        fqdn: d.domain_name,
        php_version: d.is_custom ? d.php_version : null,
        effective_version: d.php_version,
      })),
    });
  }),

  // PUT /api/v1/client/php  { version }
  http.put(/\/api\/v1\/client\/php$/, async ({ request }) => {
    const body = (await request.json()) as { version: string };
    if (!mockClientPhpConfig.supported_versions.some((v) => v.version === body.version)) {
      return problem(422, "unknown_version", `Unsupported PHP version: ${body.version}`);
    }
    mockClientPhpConfig.default_version = body.version;
    mockClientPhpConfig.supported_versions.forEach((v) => {
      v.is_default = v.version === body.version;
    });
    mockClientPhpConfig.domain_configs.forEach((d) => {
      if (!d.is_custom) d.php_version = body.version;
    });
    return HttpResponse.json({ ok: true });
  }),

  // PUT /api/v1/client/domains/:id/php  { version | null }
  http.put(/\/api\/v1\/client\/domains\/([^/]+)\/php$/, async ({ params, request }) => {
    const body = (await request.json()) as { version: string | null };
    const dom = mockClientPhpConfig.domain_configs.find((d) => d.domain_id === params[0]);
    if (!dom) return problem(404, "not_found", "No such domain");
    if (body.version !== null && !mockClientPhpConfig.supported_versions.some((v) => v.version === body.version)) {
      return problem(422, "unknown_version", `Unsupported PHP version: ${body.version}`);
    }
    dom.php_version = body.version ?? mockClientPhpConfig.default_version;
    dom.is_custom = body.version !== null && body.version !== mockClientPhpConfig.default_version;
    return HttpResponse.json({ ok: true });
  }),

  // GET/PUT /api/v1/client/php/settings, shaped like `PhpSettingsView`
  http.get(/\/api\/v1\/client\/php\/settings$/, () => {
    const ini = mockClientPhpConfig.ini_settings;
    const defaults = mockClientPhpConfig.plan_defaults;
    const asValues = (i: typeof ini) => ({
      memory_limit: i.memory_limit,
      upload_max_filesize: i.upload_max_filesize,
      post_max_size: i.post_max_size,
      max_execution_time: i.max_execution_time,
      max_input_time: i.max_input_time,
      display_errors: i.display_errors ? "On" : "Off",
      "date.timezone": i.date_timezone,
    });
    return HttpResponse.json({
      options: Object.entries(asValues(defaults)).map(([key, value]) => ({
        key,
        kind: typeof value === "number" ? "int" : key === "display_errors" ? "flag" : key === "date.timezone" ? "timezone" : "size",
        default: String(value),
        choices: [],
      })),
      values: asValues(ini),
      plan_limits: { memory_limit_mb: 512, upload_max_mb: 128, max_execution_time: 300 },
    });
  }),
  http.put(/\/api\/v1\/client\/php\/settings$/, async ({ request }) => {
    const { values } = (await request.json()) as { values: Record<string, any> };
    if (Object.keys(values).length === 0) {
      // An empty object puts every option back to its default.
      mockClientPhpConfig.ini_settings = { ...mockClientPhpConfig.plan_defaults };
      return HttpResponse.json({ ok: true });
    }
    const parseM = (v: string) => parseInt(String(v).replace(/[^0-9]/g, ""), 10) || 0;
    if (values.max_execution_time !== undefined && Number(values.max_execution_time) > 300) {
      return problem(422, "above_plan_limit", "max_execution_time is above what the plan allows (300)");
    }
    if (values.memory_limit && parseM(values.memory_limit) > 512) {
      return problem(422, "above_plan_limit", "memory_limit is above what the plan allows (512M)");
    }
    mockClientPhpConfig.ini_settings = {
      ...mockClientPhpConfig.ini_settings,
      memory_limit: values.memory_limit ?? mockClientPhpConfig.ini_settings.memory_limit,
      upload_max_filesize: values.upload_max_filesize ?? mockClientPhpConfig.ini_settings.upload_max_filesize,
      post_max_size: values.post_max_size ?? mockClientPhpConfig.ini_settings.post_max_size,
      max_execution_time: values.max_execution_time ?? mockClientPhpConfig.ini_settings.max_execution_time,
      max_input_time: values.max_input_time ?? mockClientPhpConfig.ini_settings.max_input_time,
      display_errors: values.display_errors ?? mockClientPhpConfig.ini_settings.display_errors,
      date_timezone: values["date.timezone"] ?? mockClientPhpConfig.ini_settings.date_timezone,
    };
    return HttpResponse.json({ ok: true });
  }),

  // Services: List web services (GET /api/v1/client/services)
  // TODO(api:T06.12): Replace with real backend client services endpoint
  http.get(/\/api\/v1\/client\/services$/, () => {
    const now = Date.now();
    const items = mockServices.map((srv) => {
      const recent = (mockServiceRestartsLog[srv.name] || []).filter((t) => now - t < 60000);
      return {
        ...srv,
        restarts_count_this_minute: recent.length,
      };
    });
    return HttpResponse.json({
      items,
      total: items.length,
    });
  }),

  // Services: Restart service (POST /api/v1/client/services/:serviceName/restart)
  // TODO(api:T06.12): Replace with real backend service restart endpoint
  http.post(/\/api\/v1\/client\/services\/([^/]+)\/restart$/, ({ params }) => {
    const serviceName = (params.serviceName || params[0]) as "nginx" | "php-fpm";
    const srv = mockServices.find((s) => s.name === serviceName);
    if (!srv) {
      return HttpResponse.json({ message: `Service ${serviceName} not found` }, { status: 404 });
    }

    const now = Date.now();
    const recent = (mockServiceRestartsLog[serviceName] || []).filter((t) => now - t < 60000);

    // T06.12 DoD: Rate limit 3 restarts per minute -> 429
    if (recent.length >= 3) {
      const oldest = recent[0];
      const retryAfter = Math.max(1, Math.ceil((oldest + 60000 - now) / 1000));
      return HttpResponse.json(
        {
          error: "rate_limit_exceeded",
          message: "Rate limit exceeded: You can restart this service at most 3 times per minute.",
          retry_after_seconds: retryAfter,
        },
        { status: 429 }
      );
    }

    recent.push(now);
    mockServiceRestartsLog[serviceName] = recent;
    srv.last_restarted_at = new Date().toISOString();
    srv.uptime_seconds = 0;

    return HttpResponse.json({
      success: true,
      message: `${srv.display_name} restarted successfully.`,
      service: {
        ...srv,
        restarts_count_this_minute: recent.length,
      },
    });
  }),

  // Error & suspended pages (GET /api/v1/admin/error-pages, PUT/DELETE .../:kind), shaped like `PageList`
  ...(() => {
    const KIND: Record<string, string> = {
      suspended_domain: "suspended_domain",
      suspended_user: "suspended_account",
      error_404: "not_found",
      error_502: "bad_gateway",
    };
    const view = () => ({
      items: mockPageTemplates.map((tpl) => ({ kind: KIND[tpl.type], html: tpl.content, is_custom: tpl.isCustomized })),
      max_bytes: 262144,
    });
    const find = (kind: string) => mockPageTemplates.find((t) => KIND[t.type] === kind);
    return [
      http.get(/\/api\/v1\/admin\/error-pages$/, () => HttpResponse.json(view())),
      http.put(/\/api\/v1\/admin\/error-pages\/([^/]+)$/, async ({ params, request }) => {
        const tpl = find(params[0] as string);
        if (!tpl) return problem(404, "not_found", "No such page");
        const body = (await request.json()) as { html?: string };
        if (typeof body.html === "string") {
          tpl.content = body.html;
          tpl.isCustomized = true;
        }
        return HttpResponse.json(view());
      }),
      http.delete(/\/api\/v1\/admin\/error-pages\/([^/]+)$/, ({ params }) => {
        const tpl = find(params[0] as string);
        if (!tpl) return problem(404, "not_found", "No such page");
        tpl.content = tpl.defaultContent;
        tpl.isCustomized = false;
        return new HttpResponse(null, { status: 204 });
      }),
    ];
  })(),

  // GET/POST /api/v1/client/domains/:id/previews, DELETE .../:preview_id
  http.get(/\/api\/v1\/client\/domains\/([^/]+)\/previews$/, ({ params }) => {
    const domainId = params[0] as string;
    if (!mockClientDomains.some((d) => d.id === domainId)) return problem(404, "not_found", "No such domain");
    const link = mockPreviewLinks[domainId];
    return HttpResponse.json({
      enabled: true,
      items: link?.enabled ? [{ id: link.token ?? "prv", host: new URL(link.url ?? "https://x.preview.builtpanel.net").host, expires_at: link.expires_at }] : [],
    });
  }),
  http.post(/\/api\/v1\/client\/domains\/([^/]+)\/previews$/, ({ params }) => {
    const domainId = params[0] as string;
    const domain = mockClientDomains.find((d) => d.id === domainId);
    if (!domain) return problem(404, "not_found", "No such domain");
    const token = `prv_${Math.random().toString(36).substring(2, 8)}`;
    const now = Date.now();
    const expiresAt = new Date(now + 24 * 3600 * 1000).toISOString();
    mockPreviewLinks[domainId] = {
      domain_id: domainId,
      target_domain: domain.name,
      enabled: true,
      token,
      url: `https://${token}.preview.builtpanel.net`,
      created_at: new Date(now).toISOString(),
      expires_at: expiresAt,
      ttl_hours: 24,
    };
    return HttpResponse.json({ id: token, host: `${token}.preview.builtpanel.net`, expires_at: expiresAt }, { status: 201 });
  }),
  http.delete(/\/api\/v1\/client\/domains\/([^/]+)\/previews\/([^/]+)$/, ({ params }) => {
    const link = mockPreviewLinks[params[0] as string];
    if (!link) return problem(404, "not_found", "No such preview link");
    link.enabled = false;
    return new HttpResponse(null, { status: 204 });
  }),

  // Temporary Preview Link Proxy: Test / Access Link (GET /api/v1/preview/:token)
  // TODO(api:T06.14): Replace with real backend preview proxy route
  http.get(/\/api\/v1\/preview\/([^/]+)$/, ({ params }) => {
    const token = (params.token || params[0]) as string;
    const link = Object.values(mockPreviewLinks).find((l) => l.token === token && l.enabled);
    if (!link) {
      return HttpResponse.json({ message: "Preview link not found" }, { status: 404 });
    }

    // T06.14 DoD: Expired preview link must not work (returns HTTP 410 Gone)
    const isExpired = Date.now() > new Date(link.expires_at).getTime();
    if (isExpired) {
      return HttpResponse.json(
        {
          error: "preview_link_expired",
          message: "Preview link has expired and is no longer valid.",
        },
        { status: 410 }
      );
    }

    return HttpResponse.json({
      status: 200,
      message: "Preview link active",
      target_domain: link.target_domain,
    });
  }),

  // GET /api/v1/client/databases
  http.get(/\/api\/v1\/client\/databases$/, () => {
    return HttpResponse.json({ items: mockDatabases, prefix: "demo_", limit: 5 });
  }),

  // POST /api/v1/client/databases
  http.post(/\/api\/v1\/client\/databases$/, async ({ request }) => {
    const body = (await request.json()) as { name: string; engine?: "mysql" | "postgres" };
    const name = body.name.startsWith("demo_") ? body.name : `demo_${body.name}`;
    if (mockDatabases.some((d) => d.name === name)) {
      return problem(409, "name_taken", "A database with this name exists");
    }
    const db: MockDb = {
      id: `db_${Date.now()}`,
      name,
      engine: body.engine ?? "mysql",
      status: "ready",
      size_bytes: 0,
      size_checked_at: null,
      users: [],
    };
    mockDatabases = [db, ...mockDatabases];
    return HttpResponse.json(db, { status: 201 });
  }),

  // DELETE /api/v1/client/databases/:id (202: gone when the node has removed it)
  http.delete(/\/api\/v1\/client\/databases\/([^/]+)$/, ({ params }) => {
    const id = (params[0] || params.id) as string;
    if (!mockDatabases.some((d) => d.id === id)) return problem(404, "not_found", "No such database");
    mockDatabases = mockDatabases.filter((d) => d.id !== id);
    mockDbUsers.forEach((u) => {
      u.databases = u.databases.filter((g) => g.database_id !== id);
    });
    return new HttpResponse(null, { status: 202 });
  }),

  // GET /api/v1/client/db-users
  http.get(/\/api\/v1\/client\/db-users$/, () => {
    return HttpResponse.json({ items: mockDbUsers, limit: 5 });
  }),

  // POST /api/v1/client/db-users
  http.post(/\/api\/v1\/client\/db-users$/, async ({ request }) => {
    const body = (await request.json()) as { name: string; password?: string; engine?: "mysql" | "postgres" };
    const name = body.name.startsWith("demo_") ? body.name : `demo_${body.name}`;
    if (mockDbUsers.some((u) => u.name === name)) {
      return problem(409, "name_taken", "A user with this name exists");
    }
    const user: MockDbUser = {
      id: `dbusr_${Date.now()}`,
      name,
      engine: body.engine ?? "mysql",
      status: "ready",
      databases: [],
      remote_hosts: [],
    };
    mockDbUsers = [user, ...mockDbUsers];
    return HttpResponse.json({ user, password: body.password ? null : "Generated-Once-1" }, { status: 201 });
  }),

  // DELETE /api/v1/client/db-users/:id (202)
  http.delete(/\/api\/v1\/client\/db-users\/([^/]+)$/, ({ params }) => {
    const id = (params[0] || params.id) as string;
    if (!mockDbUsers.some((u) => u.id === id)) return problem(404, "not_found", "No such user");
    mockDbUsers = mockDbUsers.filter((u) => u.id !== id);
    mockDatabases.forEach((d) => {
      d.users = d.users.filter((g) => g.user_id !== id);
    });
    return new HttpResponse(null, { status: 202 });
  }),

  // PUT /api/v1/client/db-users/:id/password
  http.put(/\/api\/v1\/client\/db-users\/([^/]+)\/password$/, ({ params }) => {
    const user = mockDbUsers.find((u) => u.id === ((params[0] || params.id) as string));
    if (!user) return problem(404, "not_found", "No such user");
    return HttpResponse.json({ user, password: null });
  }),

  // GET/PUT /api/v1/client/db-users/:id/remote-hosts
  http.get(/\/api\/v1\/client\/db-users\/([^/]+)\/remote-hosts$/, ({ params }) => {
    const user = mockDbUsers.find((u) => u.id === ((params[0] || params.id) as string));
    if (!user) return problem(404, "not_found", "No such user");
    return HttpResponse.json({ cidrs: user.remote_hosts });
  }),
  http.put(/\/api\/v1\/client\/db-users\/([^/]+)\/remote-hosts$/, async ({ params, request }) => {
    const user = mockDbUsers.find((u) => u.id === ((params[0] || params.id) as string));
    if (!user) return problem(404, "not_found", "No such user");
    const body = (await request.json()) as { cidrs: string[] };
    user.remote_hosts = body.cidrs;
    return HttpResponse.json({ cidrs: user.remote_hosts });
  }),

  // PUT/DELETE /api/v1/client/databases/:id/grants/:user_id
  http.put(/\/api\/v1\/client\/databases\/([^/]+)\/grants\/([^/]+)$/, async ({ params, request }) => {
    const db = mockDatabases.find((d) => d.id === params[0]);
    const user = mockDbUsers.find((u) => u.id === params[1]);
    if (!db || !user) return problem(404, "not_found", "No such database or user");
    const body = (await request.json()) as { access: string };
    if (!["read_only", "read_write", "all"].includes(body.access)) {
      return problem(422, "unknown_access", "The access level is read_only, read_write or all.");
    }
    db.users = [...db.users.filter((g) => g.user_id !== user.id), { user: user.name, user_id: user.id, access: body.access }];
    user.databases = [
      ...user.databases.filter((g) => g.database_id !== db.id),
      { database: db.name, database_id: db.id, access: body.access },
    ];
    return HttpResponse.json(db);
  }),
  http.delete(/\/api\/v1\/client\/databases\/([^/]+)\/grants\/([^/]+)$/, ({ params }) => {
    const db = mockDatabases.find((d) => d.id === params[0]);
    const user = mockDbUsers.find((u) => u.id === params[1]);
    if (!db || !user) return problem(404, "not_found", "No such database or user");
    db.users = db.users.filter((g) => g.user_id !== user.id);
    user.databases = user.databases.filter((g) => g.database_id !== db.id);
    return new HttpResponse(null, { status: 202 });
  }),

  // POST /api/v1/client/databases/:id/phpmyadmin
  http.post(/\/api\/v1\/client\/databases\/([^/]+)\/phpmyadmin$/, ({ params }) => {
    if (!mockDatabases.some((d) => d.id === params[0])) return problem(404, "not_found", "No such database");
    return HttpResponse.json({
      url: "https://pma.builtpanel.net/sso?token=mock",
      expires_at: new Date(Date.now() + 300 * 1000).toISOString(),
    });
  }),

  // Exports: POST /api/v1/client/databases/:id/exports (202), GET /api/v1/client/exports/:id,
  // POST /api/v1/client/exports/:id/link
  http.post(/\/api\/v1\/client\/databases\/([^/]+)\/exports$/, ({ params }) => {
    const db = mockDatabases.find((d) => d.id === params[0]);
    if (!db) return problem(404, "not_found", "No such database");
    return HttpResponse.json(
      {
        id: "exp_01",
        database: db.name,
        database_id: db.id,
        status: "queued",
        received_bytes: 0,
        created_at: new Date().toISOString(),
      },
      { status: 202 },
    );
  }),
  http.get(/\/api\/v1\/client\/exports\/([^/]+)$/, ({ params }) =>
    HttpResponse.json({
      id: params[0],
      database: "demo_wordpress",
      database_id: "db_01",
      status: "ready",
      received_bytes: 1024,
      size_bytes: 1024,
      created_at: new Date().toISOString(),
    }),
  ),
  http.post(/\/api\/v1\/client\/exports\/([^/]+)\/link$/, () =>
    HttpResponse.json({
      url: "https://downloads.builtpanel.net/dumps/demo_wordpress.sql.gz?signature=mock",
      expires_at: new Date(Date.now() + 300 * 1000).toISOString(),
    }),
  ),

  // Imports: POST /api/v1/client/databases/:id/imports, PUT .../chunks/:n, POST .../complete, GET
  http.post(/\/api\/v1\/client\/databases\/([^/]+)\/imports$/, async ({ params, request }) => {
    const db = mockDatabases.find((d) => d.id === params[0]);
    if (!db) return problem(404, "not_found", "No such database");
    const body = (await request.json()) as { size_bytes: number; gzip?: boolean };
    return HttpResponse.json(
      {
        id: "imp_01",
        database: db.name,
        database_id: db.id,
        status: "uploading",
        gzip: body.gzip ?? false,
        size_bytes: body.size_bytes,
        received_bytes: 0,
        bytes_done: 0,
        next_chunk: 0,
        max_chunk_bytes: 8 * 1024 * 1024,
        statements: 0,
        created_at: new Date().toISOString(),
      },
      { status: 201 },
    );
  }),
  http.put(/\/api\/v1\/client\/imports\/([^/]+)\/chunks\/([^/]+)$/, async ({ params, request }) => {
    const bytes = (await request.arrayBuffer()).byteLength;
    return HttpResponse.json({
      id: params[0],
      database: "demo_wordpress",
      database_id: "db_01",
      status: "uploading",
      gzip: false,
      size_bytes: bytes,
      received_bytes: bytes,
      bytes_done: 0,
      next_chunk: Number(params[1]) + 1,
      max_chunk_bytes: 8 * 1024 * 1024,
      statements: 0,
      created_at: new Date().toISOString(),
    });
  }),
  http.post(/\/api\/v1\/client\/imports\/([^/]+)\/complete$/, ({ params }) =>
    HttpResponse.json(
      {
        id: params[0],
        database: "demo_wordpress",
        database_id: "db_01",
        status: "done",
        gzip: false,
        size_bytes: 1,
        received_bytes: 1,
        bytes_done: 1,
        next_chunk: 1,
        max_chunk_bytes: 8 * 1024 * 1024,
        statements: 12,
        created_at: new Date().toISOString(),
      },
      { status: 202 },
    ),
  ),

  // Admin Databases Overview (GET /api/v1/admin/databases/overview)
  // TODO(api:T07.11): Replace with real backend admin databases overview endpoint
  http.get(/\/api\/v1\/admin\/databases\/overview$/, () => {
    return HttpResponse.json({
      summary: {
        total_databases: 48,
        total_size_bytes: 1986543616,
        total_size_formatted: "1.85 GB",
        total_users: 52,
        active_nodes: 3,
      },
      node_distributions: [
        {
          node_id: "node_01",
          node_name: "node-primary-dal",
          database_count: 24,
          total_size_bytes: 1048576000,
          total_size_formatted: "1000.0 MB",
          db_version: "11.4.3-MariaDB",
          status: "healthy",
        },
        {
          node_id: "node_02",
          node_name: "node-worker-fra",
          database_count: 16,
          total_size_bytes: 681574400,
          total_size_formatted: "650.0 MB",
          db_version: "11.4.3-MariaDB",
          status: "healthy",
        },
        {
          node_id: "node_03",
          node_name: "node-edge-sin",
          database_count: 8,
          total_size_bytes: 256393216,
          total_size_formatted: "244.5 MB",
          db_version: "11.4.3-MariaDB",
          status: "healthy",
        },
      ],
      largest_databases: [
        {
          id: "db_big_01",
          name: "ecomm_magento_prod",
          tenant_id: "usr_tenant_01",
          tenant_username: "megastore",
          node_id: "node_01",
          node_name: "node-primary-dal",
          size_bytes: 524288000,
          size_formatted: "500.0 MB",
          tables_count: 284,
          charset: "utf8mb4_unicode_ci",
          updated_at: new Date(Date.now() - 3600 * 1000).toISOString(),
        },
        {
          id: "db_big_02",
          name: "portal_lms_db",
          tenant_id: "usr_tenant_02",
          tenant_username: "edutech",
          node_id: "node_01",
          node_name: "node-primary-dal",
          size_bytes: 367001600,
          size_formatted: "350.0 MB",
          tables_count: 142,
          charset: "utf8mb4_unicode_ci",
          updated_at: new Date(Date.now() - 7200 * 1000).toISOString(),
        },
        {
          id: "db_big_03",
          name: "crm_enterprise_leads",
          tenant_id: "usr_tenant_03",
          tenant_username: "bizcorp",
          node_id: "node_02",
          node_name: "node-worker-fra",
          size_bytes: 262144000,
          size_formatted: "250.0 MB",
          tables_count: 98,
          charset: "utf8mb4_unicode_ci",
          updated_at: new Date(Date.now() - 14400 * 1000).toISOString(),
        },
        {
          id: "db_big_04",
          name: "demo_wordpress",
          tenant_id: "usr_tenant_demo",
          tenant_username: "demo",
          node_id: "node_03",
          node_name: "node-edge-sin",
          size_bytes: 24657920,
          size_formatted: "23.5 MB",
          tables_count: 14,
          charset: "utf8mb4_unicode_ci",
          updated_at: new Date(Date.now() - 86400 * 1000).toISOString(),
        },
      ],
    });
  }),

  // ===================================  // File Manager: T08.09 Handlers
  // Shaped like `EntryView`, `TrashEntry`, `TransferView` and `DiskUsageView` in openapi.json:
  // paths are relative to the account's files (`""` is the top).
  // ===================================
  ...(() => {
    interface MockEntry {
      path: string;
      kind: "file" | "dir";
      size: number;
      mode: number;
      modified_at: string;
      content?: string;
    }
    interface MockTrashEntry {
      id: string;
      entry: MockEntry;
      deleted_at: string;
    }

    const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();
    const dir = (path: string, mode = 0o755): MockEntry => ({ path, kind: "dir", size: 4096, mode, modified_at: ago(86400) });
    const file = (path: string, content: string, mode = 0o644, age = 3600): MockEntry => ({
      path,
      kind: "file",
      size: content.length,
      mode,
      modified_at: ago(age),
      content,
    });

    const mockFs: MockEntry[] = [
      dir("public_html"),
      dir("logs"),
      dir("ssl", 0o700),
      file("public_html/index.php", "<?php\necho 'Hello from Built Panel!';\n", 0o644, 1800),
      file("public_html/style.css", "body {\n  margin: 0;\n  font-family: sans-serif;\n  background: #f8fafc;\n}\n"),
      file("public_html/.htaccess", "RewriteEngine On\nRewriteRule ^index\\.php$ - [L]\n", 0o644, 86400),
      { ...file("public_html/backup.zip", "", 0o644, 172800), size: 1048576 },
      { ...file("logs/access.log", "[2026-11-13 06:00:01] 200 GET /index.php\n", 0o640, 600), size: 40960 },
    ];
    let mockTrash: MockTrashEntry[] = [];
    const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");
    const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);
    const view = (e: MockEntry) => ({ name: nameOf(e.path), kind: e.kind, size: e.size, mode: e.mode, modified_at: e.modified_at });
    const q = (request: Request, key: string) => new URL(request.url).searchParams.get(key) ?? "";
    const done = (count = 1) => HttpResponse.json({ count });

    return [
      // GET /api/v1/client/files
      http.get(/\/api\/v1\/client\/files$/, ({ request }) => {
        const path = q(request, "path");
        if (path !== "" && !mockFs.some((e) => e.path === path && e.kind === "dir")) {
          return problem(404, "not_found", "No such folder");
        }
        const items = mockFs.filter((e) => parentOf(e.path) === path);
        const offset = Number(q(request, "offset") || 0);
        const limit = Number(q(request, "limit") || 100);
        return HttpResponse.json({ items: items.slice(offset, offset + limit).map(view), total: items.length, offset, limit });
      }),

      // DELETE /api/v1/client/files?path=: to the trash
      http.delete(/\/api\/v1\/client\/files$/, ({ request }) => {
        const path = q(request, "path");
        const idx = mockFs.findIndex((e) => e.path === path);
        if (idx === -1) return problem(404, "not_found", "No such file");
        const [removed] = mockFs.splice(idx, 1);
        mockTrash.push({ id: `trash_${Date.now()}_${mockTrash.length}`, entry: removed, deleted_at: new Date().toISOString() });
        return done();
      }),

      // GET /api/v1/client/files/content, PUT (save)
      http.get(/\/api\/v1\/client\/files\/content$/, ({ request }) => {
        const found = mockFs.find((e) => e.path === q(request, "path") && e.kind === "file");
        if (!found) return problem(404, "not_found", "No such file");
        return HttpResponse.json({ binary: false, content: found.content ?? "", mtime: Date.parse(found.modified_at) / 1000, size: found.size });
      }),
      http.put(/\/api\/v1\/client\/files\/content$/, async ({ request }) => {
        const body = (await request.json()) as { path: string; content: string; create_only?: boolean };
        const found = mockFs.find((e) => e.path === body.path);
        if (found && body.create_only) return problem(409, "exists", "That name is taken");
        if (found) {
          found.content = body.content;
          found.size = body.content.length;
          found.modified_at = new Date().toISOString();
        } else {
          mockFs.push(file(body.path, body.content, 0o644, 0));
        }
        return done();
      }),

      // POST /api/v1/client/files/folders
      http.post(/\/api\/v1\/client\/files\/folders$/, async ({ request }) => {
        const body = (await request.json()) as { path: string };
        if (mockFs.some((e) => e.path === body.path)) return problem(409, "exists", "That name is taken");
        mockFs.push(dir(body.path));
        return done();
      }),

      // POST /api/v1/client/files/move (also the rename)
      http.post(/\/api\/v1\/client\/files\/move$/, async ({ request }) => {
        const body = (await request.json()) as { from: string; to: string };
        const found = mockFs.find((e) => e.path === body.from);
        if (!found) return problem(404, "not_found", "No such file");
        if (mockFs.some((e) => e.path === body.to)) return problem(409, "exists", "That name is taken");
        found.path = body.to;
        return done();
      }),

      // POST /api/v1/client/files/permissions
      http.post(/\/api\/v1\/client\/files\/permissions$/, async ({ request }) => {
        const body = (await request.json()) as { path: string; mode: number };
        const found = mockFs.find((e) => e.path === body.path);
        if (!found) return problem(404, "not_found", "No such file");
        found.mode = body.mode;
        return done();
      }),

      // POST /api/v1/client/files/archive, /extract
      http.post(/\/api\/v1\/client\/files\/archive$/, async ({ request }) => {
        const body = (await request.json()) as { dest: string; paths: string[]; format: string };
        mockFs.push({ ...file(body.dest, "", 0o644, 0), size: 524288 });
        return done(body.paths.length);
      }),
      http.post(/\/api\/v1\/client\/files\/extract$/, () => done(3)),

      // GET /api/v1/client/files/trash, POST .../:id/restore, DELETE (empty)
      http.get(/\/api\/v1\/client\/files\/trash$/, () =>
        HttpResponse.json({
          items: mockTrash.map((t) => ({ id: t.id, path: t.entry.path, kind: t.entry.kind, size: t.entry.size, deleted_at: t.deleted_at })),
        }),
      ),
      http.post(/\/api\/v1\/client\/files\/trash\/([^/]+)\/restore$/, ({ params }) => {
        const idx = mockTrash.findIndex((t) => t.id === params[0]);
        if (idx === -1) return problem(404, "not_found", "Not in the trash");
        mockFs.push(mockTrash.splice(idx, 1)[0].entry);
        return done();
      }),
      http.delete(/\/api\/v1\/client\/files\/trash$/, () => {
        const count = mockTrash.length;
        mockTrash = [];
        return done(count);
      }),

      // Uploads: POST /uploads, PUT /uploads/:id/chunks/:n, POST /uploads/:id/complete, GET /transfers/:id
      http.post(/\/api\/v1\/client\/files\/uploads$/, async ({ request }) => {
        const body = (await request.json()) as { path: string; size_bytes: number };
        return HttpResponse.json(
          {
            id: `up_${Date.now()}`,
            kind: "upload",
            path: body.path,
            status: "uploading",
            size_bytes: body.size_bytes,
            received_bytes: 0,
            next_chunk: 0,
            max_chunk_bytes: 8 * 1024 * 1024,
            created_at: new Date().toISOString(),
          },
          { status: 201 },
        );
      }),
      http.put(/\/api\/v1\/client\/files\/uploads\/([^/]+)\/chunks\/([^/]+)$/, async ({ params, request }) => {
        const bytes = (await request.arrayBuffer()).byteLength;
        return HttpResponse.json({
          id: params[0],
          kind: "upload",
          path: "",
          status: "uploading",
          received_bytes: bytes,
          next_chunk: Number(params[1]) + 1,
          max_chunk_bytes: 8 * 1024 * 1024,
          created_at: new Date().toISOString(),
        });
      }),
      http.post(/\/api\/v1\/client\/files\/uploads\/([^/]+)\/complete$/, ({ params }) =>
        HttpResponse.json(
          {
            id: params[0],
            kind: "upload",
            path: "",
            status: "done",
            received_bytes: 0,
            next_chunk: 1,
            max_chunk_bytes: 8 * 1024 * 1024,
            created_at: new Date().toISOString(),
          },
          { status: 202 },
        ),
      ),

      // GET /api/v1/client/files/usage, POST (measure now)
      http.get(/\/api\/v1\/client\/files\/usage$/, () =>
        HttpResponse.json({
          total_bytes: 1258291200,
          inodes: 12450,
          partial: false,
          measured_at: ago(1800),
          // The ten biggest folders, biggest first.
          folders: [
            { path: "public_html/wp-content/uploads", bytes: 471859200, inodes: 4200 },
            { path: "public_html/wp-content/plugins", bytes: 251658240, inodes: 6100 },
            { path: "backups", bytes: 209715200, inodes: 14 },
            { path: "public_html/wp-content/themes", bytes: 83886080, inodes: 1500 },
            { path: "public_html/wp-includes", bytes: 41943040, inodes: 900 },
            { path: "public_html/wp-admin", bytes: 31457280, inodes: 800 },
            { path: "logs", bytes: 20971520, inodes: 12 },
            { path: "public_html/cache", bytes: 10485760, inodes: 300 },
            { path: "ssl", bytes: 5242880, inodes: 8 },
            { path: "tmp", bytes: 1048576, inodes: 40 },
          ],
        }),
      ),
      http.post(/\/api\/v1\/client\/files\/usage$/, () => HttpResponse.json({ operation_id: "op_usage_1" }, { status: 202 })),
    ];
  })(),

      // ===================================      // FTP Accounts: T08.13 Handlers
      // TODO(api:T08.12): Real FTPS account management endpoints
      // ===================================
  ...(() => {
        // The shape of `FtpAccountView` in openapi.json.
        interface MockFtpAccount {
          id: string;
          name: string;
          home: string;
          quota_mb: number | null;
          enabled: boolean;
          status: "pending" | "ready" | "failed" | "deleting";
          sftp: boolean;
          ftps: boolean;
          ftp_plain: boolean;
          read_only: boolean;
        }

        const mockFtpAccounts: MockFtpAccount[] = [
          {
            id: "ftp_01",
            name: "demo_deploy",
            home: "public_html",
            quota_mb: 1024,
            enabled: true,
            status: "ready",
            sftp: true,
            ftps: true,
            ftp_plain: false,
            read_only: false,
          },
          {
            id: "ftp_02",
            name: "demo_backup",
            home: "backups",
            quota_mb: null,
            enabled: true,
            status: "ready",
            sftp: true,
            ftps: true,
            ftp_plain: false,
            read_only: false,
          },
        ];

        return [
          // GET /api/v1/client/ftp-accounts
          http.get(/\/api\/v1\/client\/ftp-accounts$/, () => {
            return HttpResponse.json({
              items: mockFtpAccounts,
              host: "ftp.demo.builtpanel.test",
              ftp_port: 21,
              sftp_port: 2222,
              prefix: "demo_",
              limit: 5,
              plain_allowed: false,
            });
          }),

          // POST /api/v1/client/ftp-accounts
          http.post(/\/api\/v1\/client\/ftp-accounts$/, async ({ request }) => {
            const body = (await request.json()) as {
              name: string;
              password?: string;
              home?: string;
              quota_mb?: number;
            };
            const name = body.name.startsWith("demo_") ? body.name : `demo_${body.name}`;
            if (mockFtpAccounts.some((a) => a.name === name)) {
              return problem(409, "name_taken", "An account with this name exists");
            }
            const account: MockFtpAccount = {
              id: `ftp_${Date.now()}`,
              name,
              home: body.home ?? "",
              quota_mb: body.quota_mb ?? null,
              enabled: true,
              status: "ready",
              sftp: true,
              ftps: true,
              ftp_plain: false,
              read_only: false,
            };
            mockFtpAccounts.push(account);
            return HttpResponse.json({ account, password: body.password ? null : "Generated-Once-1" }, { status: 201 });
          }),

          // PUT /api/v1/client/ftp-accounts/:id/password
          http.put(/\/api\/v1\/client\/ftp-accounts\/([^/]+)\/password$/, ({ params }) => {
            const found = mockFtpAccounts.find((a) => a.id === ((params[0] || params.id) as string));
            if (!found) return problem(404, "not_found", "No such account");
            return HttpResponse.json({ account: found, password: null });
          }),

          // PUT /api/v1/client/ftp-accounts/:id (quota: 0 removes the limit)
          http.put(/\/api\/v1\/client\/ftp-accounts\/([^/]+)$/, async ({ params, request }) => {
            const found = mockFtpAccounts.find((a) => a.id === ((params[0] || params.id) as string));
            if (!found) return problem(404, "not_found", "No such account");
            const body = (await request.json()) as { quota_mb?: number; enabled?: boolean };
            if (body.quota_mb !== undefined) found.quota_mb = body.quota_mb === 0 ? null : body.quota_mb;
            if (body.enabled !== undefined) found.enabled = body.enabled;
            return HttpResponse.json(found);
          }),

          // DELETE /api/v1/client/ftp-accounts/:id (202: removed when the node is done)
          http.delete(/\/api\/v1\/client\/ftp-accounts\/([^/]+)$/, ({ params }) => {
            const idx = mockFtpAccounts.findIndex((a) => a.id === ((params[0] || params.id) as string));
            if (idx === -1) return problem(404, "not_found", "No such account");
            mockFtpAccounts.splice(idx, 1);
            return new HttpResponse(null, { status: 202 });
          }),

          // ===================================          // Backup & Restore: T09.09 Handlers
          // TODO(api:T09.08): Real agent backup creation and restoration endpoints
          // ===================================
  ...(() => {
            // The shape of `BackupView` in openapi.json.
            interface MockBackup {
              id: string;
              status: "queued" | "running" | "ready" | "failed";
              trigger: "manual" | "schedule";
              size_bytes: number | null;
              error: string | null;
              created_at: string;
              finished_at: string | null;
              parts: { kind: string; name: string; database: string | null; stored_bytes: number }[];
            }

            const mockBackups: MockBackup[] = [
              {
                id: "bk_01",
                status: "ready",
                trigger: "manual",
                size_bytes: 471859200,
                error: null,
                created_at: new Date(Date.now() - 86400 * 3 * 1000).toISOString(),
                finished_at: new Date(Date.now() - 86400 * 3 * 1000 + 120000).toISOString(),
                parts: [
                  { kind: "files", name: "files", database: null, stored_bytes: 400000000 },
                  { kind: "database", name: "demo_wordpress", database: "demo_wordpress", stored_bytes: 40000000 },
                  { kind: "database", name: "demo_analytics", database: "demo_analytics", stored_bytes: 20000000 },
                  { kind: "dns", name: "dns", database: null, stored_bytes: 1859200 },
                ],
              },
              {
                id: "bk_02",
                status: "ready",
                trigger: "schedule",
                size_bytes: 52428800,
                error: null,
                created_at: new Date(Date.now() - 86400 * 1 * 1000).toISOString(),
                finished_at: new Date(Date.now() - 86400 * 1 * 1000 + 45000).toISOString(),
                parts: [{ kind: "database", name: "demo_wordpress", database: "demo_wordpress", stored_bytes: 52428800 }],
              },
            ];
            let mockSchedule: { cron: string; enabled: boolean; keep_daily: number; keep_weekly: number } | null = null;

            return [
              // GET /api/v1/client/backups
              http.get(/\/api\/v1\/client\/backups$/, () => {
                return HttpResponse.json({ items: mockBackups });
              }),

              // POST /api/v1/client/backups (202: queued)
              http.post(/\/api\/v1\/client\/backups$/, () => {
                const backup: MockBackup = {
                  id: `bk_${Date.now()}`,
                  status: "queued",
                  trigger: "manual",
                  size_bytes: null,
                  error: null,
                  created_at: new Date().toISOString(),
                  finished_at: null,
                  parts: [],
                };
                mockBackups.unshift(backup);
                return HttpResponse.json(backup, { status: 202 });
              }),

              // POST /api/v1/client/backups/:id/restore (202)
              http.post(/\/api\/v1\/client\/backups\/([^/]+)\/restore$/, async ({ params, request }) => {
                if (!mockBackups.some((b) => b.id === params[0])) return problem(404, "not_found", "No such backup");
                const body = (await request.json()) as { files?: boolean; databases?: string[]; dns?: boolean };
                return HttpResponse.json(
                  {
                    id: `rst_${Date.now()}`,
                    backup_id: params[0],
                    status: "queued",
                    files: body.files ?? false,
                    databases: body.databases ?? [],
                    dns: body.dns ?? false,
                    verify_only: false,
                    created_at: new Date().toISOString(),
                  },
                  { status: 202 },
                );
              }),

              // POST /api/v1/client/backups/:id/export (202), GET /api/v1/client/backup-exports
              http.post(/\/api\/v1\/client\/backups\/([^/]+)\/export$/, ({ params }) => {
                if (!mockBackups.some((b) => b.id === params[0])) return problem(404, "not_found", "No such backup");
                return HttpResponse.json(
                  { id: "exp_bk_1", backup_id: params[0], status: "queued", created_at: new Date().toISOString() },
                  { status: 202 },
                );
              }),
              http.get(/\/api\/v1\/client\/backup-exports$/, () =>
                HttpResponse.json({
                  items: [
                    {
                      id: "exp_bk_1",
                      backup_id: "bk_01",
                      status: "ready",
                      object: "account-export-demo.bpx",
                      size_bytes: 1048576,
                      created_at: new Date().toISOString(),
                    },
                  ],
                }),
              ),

              // DELETE /api/v1/client/backups/:id (204: being removed)
              http.delete(/\/api\/v1\/client\/backups\/([^/]+)$/, ({ params }) => {
                const idx = mockBackups.findIndex((b) => b.id === params[0]);
                if (idx === -1) return problem(404, "not_found", "No such backup");
                mockBackups.splice(idx, 1);
                return new HttpResponse(null, { status: 204 });
              }),

              // GET/PUT/DELETE /api/v1/client/backup-schedule
              http.get(/\/api\/v1\/client\/backup-schedule$/, () =>
                mockSchedule ? HttpResponse.json(mockSchedule) : problem(404, "not_found", "No schedule"),
              ),
              http.put(/\/api\/v1\/client\/backup-schedule$/, async ({ request }) => {
                mockSchedule = (await request.json()) as typeof mockSchedule;
                return HttpResponse.json(mockSchedule);
              }),
              http.delete(/\/api\/v1\/client\/backup-schedule$/, () => {
                mockSchedule = null;
                return new HttpResponse(null, { status: 204 });
              }),

              // The panel's own backups, destinations and schedule, shaped like `DestinationView`,
              // `BackupView` and the schedule in openapi.json.
  ...(() => {
                interface MockDestination {
                  id: string;
                  name: string;
                  kind: "local" | "s3" | "sftp";
                  settings: Record<string, unknown>;
                  is_default: boolean;
                  enabled: boolean;
                  has_credentials: boolean;
                  last_test_ok: boolean | null;
                  last_tested_at: string | null;
                  last_test_error: string | null;
                }
                let mockDestinations: MockDestination[] = [
                  { id: "dest_local", name: "Local Panel Storage", kind: "local", settings: { path: "/var/backups/builtpanel" }, is_default: true, enabled: true, has_credentials: false, last_test_ok: true, last_tested_at: new Date().toISOString(), last_test_error: null },
                  { id: "dest_s3_01", name: "Wasabi EU Cloud", kind: "s3", settings: { endpoint: "https://s3.eu-central-1.wasabisys.com", bucket: "builtpanel-eu", region: "eu-central-1" }, is_default: false, enabled: true, has_credentials: true, last_test_ok: true, last_tested_at: new Date().toISOString(), last_test_error: null },
                ];
                const mockSystemBackups = [
                  { id: "job_01", status: "ready", trigger: "schedule", size_bytes: 31 * 1048576, error: null as string | null, created_at: new Date(Date.now() - 86400000).toISOString(), finished_at: new Date(Date.now() - 86400000 + 30000).toISOString(), parts: [] as unknown[] },
                  { id: "job_02", status: "failed", trigger: "manual", size_bytes: null as number | null, error: "Destination unreachable" as string | null, created_at: new Date(Date.now() - 3600000).toISOString(), finished_at: new Date(Date.now() - 3500000).toISOString(), parts: [] as unknown[] },
                ];
                let mockSystemSchedule: { cron: string; enabled: boolean; keep_daily: number; keep_weekly: number } | null = {
                  cron: "0 2 * * *",
                  enabled: true,
                  keep_daily: 7,
                  keep_weekly: 4,
                };

                return [
                  http.get(/\/api\/v1\/admin\/backup-destinations$/, () => HttpResponse.json({ items: mockDestinations })),
                  http.post(/\/api\/v1\/admin\/backup-destinations$/, async ({ request }) => {
                    const body = (await request.json()) as { kind: "local" | "s3" | "sftp"; name: string; settings: Record<string, unknown>; credentials?: unknown };
                    const dest: MockDestination = {
                      id: `dest_${Date.now()}`,
                      name: body.name,
                      kind: body.kind,
                      settings: body.settings,
                      is_default: false,
                      enabled: true,
                      has_credentials: !!body.credentials,
                      last_test_ok: true,
                      last_tested_at: new Date().toISOString(),
                      last_test_error: null,
                    };
                    mockDestinations = [...mockDestinations, dest];
                    return HttpResponse.json(dest, { status: 201 });
                  }),
                  http.post(/\/api\/v1\/admin\/backup-destinations\/([^/]+)\/default$/, ({ params }) => {
                    mockDestinations = mockDestinations.map((d) => ({ ...d, is_default: d.id === params[0] }));
                    return HttpResponse.json(mockDestinations.find((d) => d.id === params[0]) ?? {});
                  }),
                  http.delete(/\/api\/v1\/admin\/backup-destinations\/([^/]+)$/, ({ params }) => {
                    mockDestinations = mockDestinations.filter((d) => d.id !== params[0]);
                    return new HttpResponse(null, { status: 204 });
                  }),

                  http.get(/\/api\/v1\/admin\/system-backups$/, () => HttpResponse.json({ items: mockSystemBackups })),
                  http.post(/\/api\/v1\/admin\/system-backups$/, () => {
                    const b = { id: `job_sys_${Date.now()}`, status: "queued", trigger: "manual", size_bytes: null, error: null, created_at: new Date().toISOString(), finished_at: null, parts: [] };
                    mockSystemBackups.unshift(b as never);
                    return HttpResponse.json(b, { status: 202 });
                  }),
                  http.delete(/\/api\/v1\/admin\/system-backups\/([^/]+)$/, ({ params }) => {
                    const i = mockSystemBackups.findIndex((b) => b.id === params[0]);
                    if (i !== -1) mockSystemBackups.splice(i, 1);
                    return new HttpResponse(null, { status: 204 });
                  }),

                  http.get(/\/api\/v1\/admin\/system-backup-schedule$/, () =>
                    mockSystemSchedule ? HttpResponse.json(mockSystemSchedule) : problem(404, "not_found", "No schedule"),
                  ),
                  http.put(/\/api\/v1\/admin\/system-backup-schedule$/, async ({ request }) => {
                    mockSystemSchedule = (await request.json()) as typeof mockSystemSchedule;
                    return HttpResponse.json(mockSystemSchedule);
                  }),
                  http.delete(/\/api\/v1\/admin\/system-backup-schedule$/, () => {
                    mockSystemSchedule = null;
                    return new HttpResponse(null, { status: 204 });
                  }),

                  // ===================================                  // CRON JOBS (TODO(api:T10.01, T10.02))
                  // ===================================
  ...(() => {
                    // The shape of `JobView` in openapi.json.
                    interface MockCronJob {
                      id: string;
                      name: string;
                      command: string;
                      schedule: string;
                      enabled: boolean;
                      timeout_seconds: number;
                      last_run_at: string | null;
                      last_status: "ok" | "failed" | "timeout" | "skipped" | null;
                      last_exit_code: number | null;
                      last_output: string | null;
                    }

                    const mockCronJobs: MockCronJob[] = [
                      {
                        id: "cron_01",
                        name: "Laravel Artisan Scheduler",
                        command: "php /home/demo/artisan schedule:run",
                        schedule: "0 * * * *",
                        enabled: true,
                        timeout_seconds: 300,
                        last_run_at: new Date(Date.now() - 1800 * 1000).toISOString(),
                        last_status: "ok",
                        last_exit_code: 0,
                        last_output: "Running scheduled command: 'cache:prune-stale-tags' [OK]\nRunning scheduled command: 'queue:restart' [OK]\nExecution completed with status 0.",
                      },
                      {
                        id: "cron_02",
                        name: "Daily Database Optimization",
                        command: "mariadb-dump-optimize --user=demo --silent",
                        schedule: "0 2 * * *",
                        enabled: true,
                        timeout_seconds: 600,
                        last_run_at: new Date(Date.now() - 25200 * 1000).toISOString(),
                        last_status: "ok",
                        last_exit_code: 0,
                        last_output: "[2026-11-13 02:00:01] Optimizing 2 databases for user 'demo'...\nTable demo_wordpress.wp_posts: OK\nTable demo_wordpress.wp_postmeta: OK\nOptimization finished.",
                      },
                      {
                        id: "cron_03",
                        name: "External Webhook Sync",
                        command: "curl -f -s https://api.partner.example/v1/webhooks/poll",
                        schedule: "*/15 * * * *",
                        enabled: false,
                        timeout_seconds: 60,
                        last_run_at: new Date(Date.now() - 86400 * 1000).toISOString(),
                        last_status: "failed",
                        last_exit_code: 22,
                        last_output: "curl: (22) The requested URL returned error: 503 Service Unavailable\nSync aborted with exit code 22.",
                      },
                    ];

                    return [
                      // GET /api/v1/client/cron-jobs
                      http.get(/\/api\/v1\/client\/cron-jobs$/, () => {
                        return HttpResponse.json({ items: mockCronJobs, limit: 10 });
                      }),

                      // POST /api/v1/client/cron-jobs
                      http.post(/\/api\/v1\/client\/cron-jobs$/, async ({ request }) => {
                        const body = (await request.json()) as any;
                        const parts = String(body.schedule || "").trim().split(/\s+/);

                        // The backend refuses schedules that fire more often than every 5 minutes.
                        if (["*", "*/1", "*/2", "*/3", "*/4"].includes(parts[0])) {
                          return problem(422, "too_often", "The schedule may not run more often than every 5 minutes");
                        }
                        if (mockCronJobs.some((j) => j.name === body.name)) {
                          return problem(409, "name_taken", "A job with this name exists");
                        }

                        const newJob: MockCronJob = {
                          id: `cron_${Date.now()}`,
                          name: body.name || "Custom Job",
                          command: body.command,
                          schedule: body.schedule || "0 * * * *",
                          enabled: body.enabled ?? true,
                          timeout_seconds: body.timeout_seconds || 300,
                          last_run_at: null,
                          last_status: null,
                          last_exit_code: null,
                          last_output: null,
                        };
                        mockCronJobs.unshift(newJob);
                        return HttpResponse.json(newJob, { status: 201 });
                      }),

                      // PUT /api/v1/client/cron-jobs/:id (replaces every field)
                      http.put(/\/api\/v1\/client\/cron-jobs\/([^/]+)$/, async ({ params, request }) => {
                        const id = (params[0] || params.id) as string;
                        const job = mockCronJobs.find((j) => j.id === id);
                        if (!job) return problem(404, "not_found", "No such job");
                        const body = (await request.json()) as any;
                        Object.assign(job, {
                          name: body.name,
                          command: body.command,
                          schedule: body.schedule,
                          enabled: body.enabled,
                          timeout_seconds: body.timeout_seconds,
                        });
                        return HttpResponse.json(job);
                      }),

                      // POST /api/v1/client/cron-jobs/:id/run (202, no body)
                      http.post(/\/api\/v1\/client\/cron-jobs\/([^/]+)\/run$/, ({ params }) => {
                        const id = (params[0] || params.id) as string;
                        const job = mockCronJobs.find((j) => j.id === id);
                        if (!job) return problem(404, "not_found", "No such job");
                        job.last_run_at = new Date().toISOString();
                        job.last_status = "ok";
                        job.last_exit_code = 0;
                        return new HttpResponse(null, { status: 202 });
                      }),

                      // DELETE /api/v1/client/cron-jobs/:id
                      http.delete(/\/api\/v1\/client\/cron-jobs\/([^/]+)$/, ({ params }) => {
                        const id = (params[0] || params.id) as string;
                        const idx = mockCronJobs.findIndex((j) => j.id === id);
                        if (idx === -1) return problem(404, "not_found", "No such job");
                        mockCronJobs.splice(idx, 1);
                        return new HttpResponse(null, { status: 204 });
                      }),

                      // ===================================                      // LOG VIEWER (TODO(api:T10.04))
                      // ===================================
  ...(() => {
                        const mockDomainOverviews = [
                          {
                            domain: "demo.builtpanel.test",
                            access_log_size_bytes: 8945600,
                            access_log_size_formatted: "8.5 MB",
                            error_log_size_bytes: 421000,
                            error_log_size_formatted: "411 KB",
                            php_error_log_size_bytes: 184500,
                            php_error_log_size_formatted: "180 KB",
                            last_updated_at: new Date().toISOString(),
                          },
                          {
                            domain: "store.builtpanel.test",
                            access_log_size_bytes: 15420000,
                            access_log_size_formatted: "14.7 MB",
                            error_log_size_bytes: 120500,
                            error_log_size_formatted: "117 KB",
                            php_error_log_size_bytes: 52100,
                            php_error_log_size_formatted: "50 KB",
                            last_updated_at: new Date().toISOString(),
                          },
                          {
                            domain: "api.builtpanel.test",
                            access_log_size_bytes: 28450000,
                            access_log_size_formatted: "27.1 MB",
                            error_log_size_bytes: 84000,
                            error_log_size_formatted: "82 KB",
                            php_error_log_size_bytes: 12000,
                            php_error_log_size_formatted: "11 KB",
                            last_updated_at: new Date().toISOString(),
                          },
                        ];

                        const generateSeedLogs = (domain: string, type: string) => {
                          const entries: any[] = [];
                          const targetDomain = domain === "all" ? "demo.builtpanel.test" : domain;
                          const baseTime = Date.now() - 3600 * 1000 * 4;

                          if (type === "access") {
                            const paths = [
                              "/",
                              "/wp-login.php",
                              "/api/v1/health",
                              "/products/12",
                              "/cart/checkout",
                              "/assets/app.css",
                              "/assets/vendor.js",
                              "/favicon.ico",
                              "/robots.txt",
                              "/admin/settings",
                            ];
                            const methods = ["GET", "GET", "POST", "GET", "PUT", "DELETE"];
                            const statuses = [200, 200, 201, 301, 304, 404, 500];

                            for (let i = 1; i <= 250; i++) {
                              const time = new Date(baseTime + i * 55000).toISOString();
                              const path = paths[i % paths.length];
                              const method = methods[i % methods.length];
                              const status = statuses[i % statuses.length];
                              const ip = `192.168.1.${(i % 150) + 10}`;

                              entries.push({
                                id: `log_acc_${i}`,
                                lineNumber: i,
                                timestamp: time,
                                type: "access",
                                domain: targetDomain,
                                status_code: status,
                                level: status >= 500 ? "error" : status >= 400 ? "warn" : "info",
                                method,
                                path,
                                ip,
                                message: `${method} ${path} HTTP/2.0 ${status} - ${ip}`,
                                raw: `${time} [${targetDomain}] ${ip} "${method} ${path} HTTP/2.0" ${status} 4280 "Mozilla/5.0"`,
                              });
                            }
                          } else if (type === "error") {
                            const errorMessages = [
                              "[error] connect() failed (111: Connection refused) while connecting to upstream",
                              "[warn] directory index of \"/var/www/html/\" is forbidden",
                              "[error] open() \"/var/www/demo/public_html/.env\" failed (13: Permission denied)",
                              "[error] upstream timed out (110: Connection timed out) while reading response header",
                              "[error] client closed connection while SSL handshaking",
                            ];

                            for (let i = 1; i <= 50; i++) {
                              const time = new Date(baseTime + i * 280000).toISOString();
                              const msg = errorMessages[i % errorMessages.length];
                              entries.push({
                                id: `log_err_${i}`,
                                lineNumber: i,
                                timestamp: time,
                                type: "error",
                                domain: targetDomain,
                                level: msg.includes("[warn]") ? "warn" : "error",
                                message: msg,
                                raw: `${time} [${targetDomain}] ${msg}, client: 192.168.1.45, server: ${targetDomain}`,
                              });
                            }
                          } else {
                            // PHP error log
                            const phpErrors = [
                              "PHP Fatal error:  Uncaught TypeError: Argument 1 passed to App\\Services\\Payment::process() must be of the type string, null given in /var/www/demo/app/Services/Payment.php:64",
                              "PHP Warning:  Undefined array key \"session_token\" in /var/www/demo/bootstrap.php on line 28",
                              "PHP Notice:  session_start(): Ignoring session_start() because a session is already active in /var/www/demo/index.php on line 12",
                              "PHP Deprecated:  Creation of dynamic property App\\Models\\User::$temp_auth is deprecated in /var/www/demo/app/Models/User.php:89",
                            ];

                            for (let i = 1; i <= 40; i++) {
                              const time = new Date(baseTime + i * 360000).toISOString();
                              const msg = phpErrors[i % phpErrors.length];
                              const isFatal = msg.includes("Fatal");
                              const isWarn = msg.includes("Warning");

                              entries.push({
                                id: `log_php_${i}`,
                                lineNumber: i,
                                timestamp: time,
                                type: "php",
                                domain: targetDomain,
                                level: isFatal ? "error" : isWarn ? "warn" : "info",
                                message: msg,
                                raw: `[${time.slice(0, 19).replace("T", " ")} UTC] ${msg}`,
                              });
                            }
                          }

                          return entries;
                        };

                        return [
                          // GET /api/v1/client/logs/overview (TODO(api:T10.04))
                          // GET /api/v1/client/logs?source=web_access|web_error|php_error&since_ms=&lines=
                          http.get(/\/api\/v1\/client\/logs$/, ({ request }) => {
                            const url = new URL(request.url);
                            const source = url.searchParams.get("source") || "web_access";
                            const since = Number(url.searchParams.get("since_ms") || 0);
                            const type = source === "web_error" ? "error" : source === "php_error" ? "php" : "access";
                            const lines = generateSeedLogs("all", type)
                              .map((e: any) => ({ at_ms: Date.parse(e.timestamp), text: e.raw as string }))
                              .filter((l: { at_ms: number }) => l.at_ms > since);
                            return HttpResponse.json({ lines, truncated: false });
                          }),

                          // ===================================                          // ADMIN SYSTEM LOGS (TODO(api:T10.07))
                          // ===================================
  ...(() => {
                            const mockServiceStats = [
                              {
                                service: "control-plane",
                                name: "Control Plane",
                                lines_count: 1450,
                                size_formatted: "14.2 MB",
                                error_count: 2,
                                last_entry_at: new Date().toISOString(),
                              },
                              {
                                service: "agent",
                                name: "Node Agent",
                                lines_count: 3200,
                                size_formatted: "28.5 MB",
                                error_count: 1,
                                last_entry_at: new Date().toISOString(),
                              },
                              {
                                service: "caddy",
                                name: "Caddy Server",
                                lines_count: 8400,
                                size_formatted: "64.1 MB",
                                error_count: 8,
                                last_entry_at: new Date().toISOString(),
                              },
                              {
                                service: "auth",
                                name: "Auth & Security",
                                lines_count: 950,
                                size_formatted: "4.8 MB",
                                error_count: 0,
                                last_entry_at: new Date().toISOString(),
                              },
                            ];

                            const generateSystemLogs = (service: string, node: string, levelFilter: string) => {
                              const entries: any[] = [];
                              const baseTime = Date.now() - 3600 * 1000 * 6;

                              const serviceMessages: Record<string, { level: string; component: string; msg: string }[]> = {
                                "control-plane": [
                                  { level: "info", component: "http", msg: "HTTP 200 OK GET /api/v1/admin/overview in 12ms" },
                                  { level: "info", component: "db", msg: "PostgreSQL pool connected: 10 active connections" },
                                  { level: "info", component: "worker", msg: "Queue job [SyncCronJobs] finished in 240ms" },
                                  { level: "warn", component: "cache", msg: "Redis connection latency spike: 45ms" },
                                  { level: "error", component: "backup", msg: "Backup job #14 failed: SFTP timeout on remote host" },
                                  { level: "info", component: "auth", msg: "Session validated for admin UID 1" },
                                ],
                                agent: [
                                  { level: "info", component: "cgroup", msg: "cgroup v2 stats collected for 16 tenant containers" },
                                  { level: "info", component: "net", msg: "iptables NAT rules synchronized for external traffic" },
                                  { level: "warn", component: "disk", msg: "Inodes usage exceeded 75% on /var/lib/docker" },
                                  { level: "error", component: "container", msg: "Container demo_php74 exited with status 137 (OOM)" },
                                  { level: "info", component: "runner", msg: "Cron runner executed job cron_01 successfully" },
                                ],
                                caddy: [
                                  { level: "info", component: "tls", msg: "TLS certificate renewed successfully for *.builtpanel.test" },
                                  { level: "info", component: "http", msg: "Handled request GET / HTTP/2.0 status=200 remote=192.168.1.10" },
                                  { level: "warn", component: "proxy", msg: "Upstream backend 127.0.0.1:9000 took > 2500ms to respond" },
                                  { level: "error", component: "tls", msg: "ACME challenge failed for invalid-domain.test: NXDOMAIN" },
                                  { level: "info", component: "admin", msg: "Caddy configuration reload successful" },
                                ],
                                auth: [
                                  { level: "info", component: "session", msg: "Admin user logged in successfully from 192.168.1.5" },
                                  { level: "info", component: "rbac", msg: "Checked permission 'system.logs.read': GRANTED" },
                                  { level: "warn", component: "ratelimit", msg: "Rate limit threshold reached for IP 192.168.1.99 (50 req/min)" },
                                  { level: "info", component: "audit", msg: "Audit log entry created for user action: DOMAIN_ADD" },
                                ],
                              };

                              const templates = serviceMessages[service] || serviceMessages["control-plane"];

                              for (let i = 1; i <= 150; i++) {
                                const tmpl = templates[i % templates.length];
                                if (levelFilter !== "all" && tmpl.level !== levelFilter) {
                                  continue;
                                }

                                const time = new Date(baseTime + i * 140000).toISOString();
                                entries.push({
                                  id: `sys_${service}_${i}`,
                                  lineNumber: i,
                                  timestamp: time,
                                  service,
                                  node,
                                  level: tmpl.level,
                                  component: tmpl.component,
                                  message: tmpl.msg,
                                  raw: `${time} [${node}] [${service}] [${tmpl.level.toUpperCase()}] [${tmpl.component}] ${tmpl.msg}`,
                                });
                              }

                              return entries;
                            };

                            return [
                              // GET /api/v1/admin/system-logs?level&since_ms&search&lines, shaped like `SystemLogList`
                              http.get(/\/api\/v1\/admin\/system-logs$/, ({ request }) => {
                                const url = new URL(request.url);
                                const level = url.searchParams.get("level") || "all";
                                const since = Number(url.searchParams.get("since_ms") || 0);
                                const logs = generateSystemLogs("control-plane", "node-01", level)
                                  .map((e: any) => ({ at_ms: Date.parse(e.timestamp), level: String(e.level).toUpperCase(), target: e.component, text: e.message }))
                                  .filter((l: { at_ms: number }) => l.at_ms > since);
                                return HttpResponse.json({ items: logs });
                              }),

                              // GET /api/v1/admin/nodes/:id/services (the core services a node reports)
                              http.get(/\/api\/v1\/admin\/nodes\/([^/]+)\/services$/, () => {
                                return HttpResponse.json({
                                  items: ["database", "phpmyadmin", "sftp"].map((name) => ({
                                    name,
                                    state: "running",
                                    detail: "",
                                    since: Math.floor(Date.now() / 1000) - 86400,
                                    checked_at: Math.floor(Date.now() / 1000),
                                  })),
                                });
                              }),

                              // GET /api/v1/admin/nodes/:id/services/:name/log
                              http.get(/\/api\/v1\/admin\/nodes\/([^/]+)\/services\/([^/]+)\/log$/, ({ params }) => {
                                const now = Date.now();
                                return HttpResponse.json({
                                  lines: Array.from({ length: 40 }).map((_, i) => ({
                                    at_ms: now - (40 - i) * 1000,
                                    text: `${params[1]}: line ${i + 1}`,
                                  })),
                                  truncated: false,
                                });
                              }),

                              // ===================================                              // USAGE ANALYTICS (TODO(api:T10.09))
                              // ===================================
  ...(() => {
                                const generateHistoryPoints = (range: string, isNode = false) => {
                                  const points: any[] = [];
                                  const count = range === "24h" ? 24 : range === "7d" ? 28 : 30;
                                  const now = Date.now();
                                  const stepMs =
                                    range === "24h"
                                      ? 3600 * 1000
                                      : range === "7d"
                                      ? (7 * 86400 * 1000) / count
                                      : (30 * 86400 * 1000) / count;

                                  for (let i = count - 1; i >= 0; i--) {
                                    const time = new Date(now - i * stepMs);
                                    const timeLabel =
                                      range === "24h"
                                        ? `${time.getHours().toString().padStart(2, "0")}:00`
                                        : `${time.getMonth() + 1}/${time.getDate()}`;

                                    const baseCpu = isNode ? 32 : 16;
                                    const cpuNoise = Math.sin(i / 2) * 12 + (Math.random() * 8 - 4);
                                    const cpu = Math.max(5, Math.min(95, Math.round((baseCpu + cpuNoise) * 10) / 10));

                                    const baseRam = isNode ? 46 : 560;
                                    const ramNoise = Math.cos(i / 3) * (isNode ? 5 : 80);
                                    const ram = Math.max(10, Math.round(baseRam + ramNoise));

                                    const baseDisk = isNode ? 820 : 3.5;
                                    const diskGrowth = ((count - i) / count) * (isNode ? 25 : 0.4);
                                    const disk = Math.round((baseDisk + diskGrowth) * 10) / 10;

                                    const netIn = Math.round(180 + Math.sin(i) * 120 + Math.random() * 40);
                                    const netOut = Math.round(240 + Math.cos(i) * 160 + Math.random() * 60);

                                    points.push({
                                      timestamp: time.toISOString(),
                                      label: timeLabel,
                                      cpu_percent: cpu,
                                      ram_mb: isNode ? ram * 1024 : ram,
                                      ram_percent: isNode ? Math.round((ram / 128) * 100) : Math.round((ram / 2048) * 100),
                                      disk_gb: disk,
                                      disk_percent: isNode ? Math.round((disk / 2000) * 100) : Math.round((disk / 10.0) * 100),
                                      net_in_kbps: isNode ? netIn * 12 : netIn,
                                      net_out_kbps: isNode ? netOut * 15 : netOut,
                                    });
                                  }

                                  return points;
                                };

                                return [
                                  // GET /api/v1/client/usage?range=, shaped like `UsageView`
                                  http.get(/\/api\/v1\/client\/usage$/, ({ request }) => {
                                    const url = new URL(request.url);
                                    const range = (url.searchParams.get("range") || "24h") as "24h" | "7d" | "30d";
                                    const history = generateHistoryPoints(range, false);
                                    const step = range === "24h" ? 3600 : range === "7d" ? 21600 : 86400;
                                    return HttpResponse.json({
                                      range,
                                      step_seconds: step,
                                      limits: { cpu_millis: 2000, memory_bytes: 2048 * 1048576, pids: 256 },
                                      points: history.map((h: any) => ({
                                        at: Math.floor(Date.parse(h.timestamp) / 1000),
                                        cpu_millis: Math.round(h.cpu_percent * 20),
                                        memory_bytes: Math.round(h.ram_mb * 1048576),
                                        io_read_bytes: 0,
                                        io_write_bytes: 0,
                                        net_rx_bytes: Math.round((h.net_in_kbps * 1000 * step) / 8),
                                        net_tx_bytes: Math.round((h.net_out_kbps * 1000 * step) / 8),
                                        pids: 12,
                                      })),
                                    });
                                  }),

                                  // GET /api/v1/admin/usage/nodes/:id?range=, shaped like `NodeUsageView`
                                  http.get(/\/api\/v1\/admin\/usage\/nodes\/([^/]+)$/, ({ request }) => {
                                    const range = (new URL(request.url).searchParams.get("range") || "24h") as "24h" | "7d" | "30d";
                                    const history = generateHistoryPoints(range, true);
                                    const step = range === "24h" ? 3600 : range === "7d" ? 21600 : 86400;
                                    return HttpResponse.json({
                                      step_seconds: step,
                                      points: history.map((h: any) => ({
                                        at: Math.floor(Date.parse(h.timestamp) / 1000),
                                        containers: 12,
                                        cpu_busy_millis: Math.round((h.cpu_percent / 100) * step * 1000),
                                        load1_x100: 120,
                                        memory_total_bytes: 128 * 1024 ** 3,
                                        memory_used_bytes: Math.round(h.ram_mb * 1048576),
                                        disk_total_bytes: 2000 * 1024 ** 3,
                                        disk_used_bytes: Math.round(h.disk_gb * 1024 ** 3),
                                      })),
                                    });
                                  }),

                                  // GET /api/v1/admin/usage/top?by=&range=&limit=, shaped like `TopItem`
                                  http.get(/\/api\/v1\/admin\/usage\/top$/, ({ request }) => {
                                    const by = new URL(request.url).searchParams.get("by") || "cpu";
                                    const GB = 1024 ** 3;
                                    const rows = [
                                      { tenant_id: "t_03", username: "shopify-clone", cpu_millis: 882, memory_bytes: 7200 * 1048576, net_bytes: 620 * GB, io_bytes: 84 * GB },
                                      { tenant_id: "t_01", username: "acme-corp", cpu_millis: 784, memory_bytes: 3400 * 1048576, net_bytes: 412 * GB, io_bytes: 42 * GB },
                                      { tenant_id: "t_02", username: "demo", cpu_millis: 456, memory_bytes: 1120 * 1048576, net_bytes: 24 * GB, io_bytes: 4 * GB },
                                      { tenant_id: "t_06", username: "video-stream", cpu_millis: 320, memory_bytes: 2100 * 1048576, net_bytes: 890 * GB, io_bytes: 38 * GB },
                                    ];
                                    const key = by === "memory" ? "memory_bytes" : by === "network" ? "net_bytes" : by === "io" ? "io_bytes" : "cpu_millis";
                                    return HttpResponse.json({ items: [...rows].sort((x, y) => (y as any)[key] - (x as any)[key]) });
                                  }),

                                  // ===================================                                  // PROCESS MANAGER (TODO(api:T10.14))
                                  // ===================================
  ...(() => {
                                    interface MockProcess {
                                      pid: number;
                                      ppid: number;
                                      user: string;
                                      cpu_percent: number;
                                      mem_percent: number;
                                      mem_rss_mb: number;
                                      vsz_mb: number;
                                      stat: string;
                                      start_time: string;
                                      cpu_time: string;
                                      command: string;
                                      is_killable: boolean;
                                      tenant?: string;
                                      node?: string;
                                    }

                                    const mockClientProcesses: MockProcess[] = [
                                      {
                                        pid: 1,
                                        ppid: 0,
                                        user: "root",
                                        cpu_percent: 0.1,
                                        mem_percent: 0.8,
                                        mem_rss_mb: 16.2,
                                        vsz_mb: 42.0,
                                        stat: "Ss",
                                        start_time: "Nov10",
                                        cpu_time: "0:02.14",
                                        command: "php-fpm: master process (/etc/php/8.2/fpm/php-fpm.conf)",
                                        is_killable: false,
                                      },
                                      {
                                        pid: 14,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 14.2,
                                        mem_percent: 2.3,
                                        mem_rss_mb: 48.5,
                                        vsz_mb: 120.4,
                                        stat: "R",
                                        start_time: "10:14",
                                        cpu_time: "0:18.40",
                                        command: "php-fpm: pool www",
                                        is_killable: true,
                                      },
                                      {
                                        pid: 15,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 4.8,
                                        mem_percent: 2.1,
                                        mem_rss_mb: 42.1,
                                        vsz_mb: 118.0,
                                        stat: "S",
                                        start_time: "10:14",
                                        cpu_time: "0:05.12",
                                        command: "php-fpm: pool www",
                                        is_killable: true,
                                      },
                                      {
                                        pid: 88,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 2.4,
                                        mem_percent: 4.1,
                                        mem_rss_mb: 84.0,
                                        vsz_mb: 210.0,
                                        stat: "S",
                                        start_time: "Nov10",
                                        cpu_time: "1:42.10",
                                        command: "mysqld: tenant socket proxy",
                                        is_killable: true,
                                      },
                                      {
                                        pid: 142,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 18.5,
                                        mem_percent: 3.2,
                                        mem_rss_mb: 64.2,
                                        vsz_mb: 145.0,
                                        stat: "R",
                                        start_time: "09:30",
                                        cpu_time: "2:15.30",
                                        command: "python3 /home/demo/scripts/feed_worker.py",
                                        is_killable: true,
                                      },
                                      {
                                        pid: 204,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 0.1,
                                        mem_percent: 0.6,
                                        mem_rss_mb: 12.0,
                                        vsz_mb: 32.0,
                                        stat: "S",
                                        start_time: "02:00",
                                        cpu_time: "0:00.45",
                                        command: "/usr/sbin/cron -f",
                                        is_killable: true,
                                      },
                                    ];

                                    const mockAdminProcesses: MockProcess[] = [
                                      {
                                        pid: 1,
                                        ppid: 0,
                                        user: "root",
                                        cpu_percent: 0.2,
                                        mem_percent: 0.4,
                                        mem_rss_mb: 24.0,
                                        vsz_mb: 180.0,
                                        stat: "Ss",
                                        start_time: "Nov01",
                                        cpu_time: "12:40.12",
                                        command: "/sbin/init splash",
                                        is_killable: false,
                                        tenant: undefined,
                                        node: "node-01",
                                      },
                                      {
                                        pid: 412,
                                        ppid: 1,
                                        user: "root",
                                        cpu_percent: 1.8,
                                        mem_percent: 1.2,
                                        mem_rss_mb: 148.0,
                                        vsz_mb: 840.0,
                                        stat: "S",
                                        start_time: "Nov01",
                                        cpu_time: "48:12.30",
                                        command: "/usr/bin/dockerd -H fd://",
                                        is_killable: true,
                                        tenant: undefined,
                                        node: "node-01",
                                      },
                                      {
                                        pid: 580,
                                        ppid: 1,
                                        user: "caddy",
                                        cpu_percent: 3.4,
                                        mem_percent: 0.9,
                                        mem_rss_mb: 112.0,
                                        vsz_mb: 420.0,
                                        stat: "S",
                                        start_time: "Nov01",
                                        cpu_time: "32:10.45",
                                        command: "caddy run --config /etc/caddy/Caddyfile",
                                        is_killable: true,
                                        tenant: undefined,
                                        node: "node-01",
                                      },
                                      {
                                        pid: 1400,
                                        ppid: 1,
                                        user: "demo",
                                        cpu_percent: 14.2,
                                        mem_percent: 2.3,
                                        mem_rss_mb: 48.5,
                                        vsz_mb: 120.4,
                                        stat: "R",
                                        start_time: "10:14",
                                        cpu_time: "0:18.40",
                                        command: "container(demo): php-fpm: pool www",
                                        is_killable: true,
                                        tenant: "demo",
                                        node: "node-01",
                                      },
                                      {
                                        pid: 2200,
                                        ppid: 1,
                                        user: "acme",
                                        cpu_percent: 42.1,
                                        mem_percent: 8.5,
                                        mem_rss_mb: 420.0,
                                        vsz_mb: 980.0,
                                        stat: "R",
                                        start_time: "08:12",
                                        cpu_time: "14:20.10",
                                        command: "container(acme): node /var/www/acme/server.js",
                                        is_killable: true,
                                        tenant: "acme",
                                        node: "node-01",
                                      },
                                    ];

                                    return [
                                      // GET /api/v1/client/processes, shaped like `ProcessView` in openapi.json
                                      http.get(/\/api\/v1\/client\/processes$/, () => {
                                        return HttpResponse.json({
                                          items: mockClientProcesses.map((p) => ({
                                            container: p.command.includes("php") ? "demo-php" : "demo-web",
                                            pid: p.pid,
                                            uid: 1000,
                                            state: p.stat.slice(0, 1),
                                            command: p.command,
                                          })),
                                        });
                                      }),

                                      // POST /api/v1/client/processes/:container/:pid/signal
                                      http.post(/\/api\/v1\/client\/processes\/([^/]+)\/([^/]+)\/signal$/, ({ params }) => {
                                        const pid = Number(params[1]);
                                        const idx = mockClientProcesses.findIndex((p) => p.pid === pid);
                                        if (idx === -1) return problem(404, "not_found", "No such process");
                                        mockClientProcesses.splice(idx, 1);
                                        return HttpResponse.json({ outcome: "terminated", pid });
                                      }),

                                      // GET /api/v1/admin/processes (TODO(api:T10.14))
                                      http.get(/\/api\/v1\/admin\/processes$/, ({ request }) => {
                                        const url = new URL(request.url);
                                        const tenant = url.searchParams.get("tenant") || "all";
                                        let filtered = mockAdminProcesses;
                                        if (tenant === "host") {
                                          filtered = filtered.filter((p) => !p.tenant);
                                        } else if (tenant !== "all") {
                                          filtered = filtered.filter((p) => p.tenant === tenant);
                                        }
                                        return HttpResponse.json(filtered);
                                      }),

                                      // GET /api/v1/admin/processes/stats (TODO(api:T10.14))
                                      http.get(/\/api\/v1\/admin\/processes\/stats$/, () => {
                                        const totalCpu = mockAdminProcesses.reduce((acc, p) => acc + p.cpu_percent, 0);
                                        const totalMem = mockAdminProcesses.reduce((acc, p) => acc + p.mem_rss_mb, 0);
                                        return HttpResponse.json({
                                          total_processes: mockAdminProcesses.length + 180,
                                          running_processes: 6,
                                          sleeping_processes: 178,
                                          zombie_processes: 1,
                                          total_cpu_percent: Math.round(totalCpu * 10) / 10,
                                          total_mem_rss_mb: Math.round(totalMem * 10) / 10,
                                        });
                                      }),

                                      // POST /api/v1/admin/processes/:id/kill (TODO(api:T10.14))
                                      http.post(/\/api\/v1\/admin\/processes\/([^/]+)\/kill$/, ({ params }) => {
                                        const pid = Number(params[0] || params.id);
                                        const idx = mockAdminProcesses.findIndex((p) => p.pid === pid);
                                        if (idx !== -1) {
                                          mockAdminProcesses.splice(idx, 1);
                                        }
                                        return HttpResponse.json({ success: true, message: `Admin signal sent to ${pid}` });
                                      }),

                                      ...(() => {
                                        const mockServerOverview = {
                                          node_id: "node-1",
                                          hostname: "node-1.builtpanel.internal",
                                          os: "Ubuntu 24.04.1 LTS",
                                          kernel: "6.8.0-45-generic",
                                          uptime_seconds: 432000,
                                          uptime_human: "5 days, 4 hours",
                                          current_time: "2026-11-13 11:45:00 UTC",
                                          timezone: "UTC",
                                          memory: {
                                            total_mb: 8192,
                                            free_mb: 1840,
                                            available_mb: 4620,
                                            buffers_mb: 320,
                                            cached_mb: 2460,
                                          },
                                          swap: {
                                            total_mb: 4096,
                                            used_mb: 256,
                                            free_mb: 3840,
                                            swappiness: 60,
                                          },
                                        };

                                        const mockSshConfig = {
                                          port: 22,
                                          permit_root_login: "no" as const,
                                          password_authentication: false,
                                          pubkey_authentication: true,
                                          max_auth_tries: 3,
                                          authorized_keys_count: 2,
                                          ciphers: ["chacha20-poly1305@openssh.com", "aes256-gcm@openssh.com"],
                                          security_rating: "hardened" as const,
                                        };

                                        const mockServerAuditEvents: Array<{
                                          id: string;
                                          action: string;
                                          actor: string;
                                          details: string;
                                          created_at: string;
                                          status: "success" | "failure";
                                        }> = [
                                          {
                                            id: "srv-aud-01",
                                            action: "server.set_timezone",
                                            actor: "admin (system)",
                                            details: "Timezone set to UTC via priv-helper",
                                            created_at: "2026-11-12 10:15:22 UTC",
                                            status: "success",
                                          },
                                          {
                                            id: "srv-aud-02",
                                            action: "server.configure_swap",
                                            actor: "admin (system)",
                                            details: "Swap size resized to 4096 MB, swappiness 60",
                                            created_at: "2026-11-12 14:30:10 UTC",
                                            status: "success",
                                          },
                                        ];

                                        return [

                                          ...(() => {
                                            const mockWafEvents: Array<{
                                              id: string;
                                              timestamp: string;
                                              client_ip: string;
                                              country: string;
                                              domain: string;
                                              http_method: string;
                                              uri: string;
                                              rule_id: number;
                                              rule_message: string;
                                              category: "sqli" | "xss" | "rce" | "scanner" | "protocol" | "generic";
                                              severity: "CRITICAL" | "HIGH" | "MEDIUM" | "LOW";
                                              action: "blocked" | "detected";
                                              matched_data?: string;
                                              user_agent?: string;
                                              tags?: string[];
                                            }> = [
                                              {
                                                id: "waf-ev-01",
                                                timestamp: "2026-11-13 11:20:14",
                                                client_ip: "198.51.100.42",
                                                country: "US",
                                                domain: "example.com",
                                                http_method: "POST",
                                                uri: "/wp-login.php",
                                                rule_id: 942100,
                                                rule_message: "SQL Injection Attack: SQL Operator Detected",
                                                category: "sqli" as const,
                                                severity: "CRITICAL" as const,
                                                action: "blocked" as const,
                                                matched_data: "ARGS:log -> admin' OR '1'='1",
                                                user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                                                tags: ["OWASP_CRS/WEB_ATTACK/SQL_INJECTION", "attack-sqli"],
                                              },
                                              {
                                                id: "waf-ev-02",
                                                timestamp: "2026-11-13 11:32:05",
                                                client_ip: "203.0.113.88",
                                                country: "BD",
                                                domain: "example.com",
                                                http_method: "GET",
                                                uri: "/search?q=<script>alert(1)</script>",
                                                rule_id: 941100,
                                                rule_message: "XSS Attack Detected via libinjection",
                                                category: "xss" as const,
                                                severity: "HIGH" as const,
                                                action: "blocked" as const,
                                                matched_data: "ARGS:q -> <script>alert(1)</script>",
                                                user_agent: "curl/8.5.0",
                                                tags: ["OWASP_CRS/WEB_ATTACK/XSS", "attack-xss"],
                                              },
                                              {
                                                id: "waf-ev-03",
                                                timestamp: "2026-11-13 11:45:50",
                                                client_ip: "192.0.2.15",
                                                country: "DE",
                                                domain: "example.com",
                                                http_method: "POST",
                                                uri: "/api/checkout",
                                                rule_id: 920350,
                                                rule_message: "Host header is a numeric IP address",
                                                category: "protocol" as const,
                                                severity: "MEDIUM" as const,
                                                action: "detected" as const,
                                                matched_data: "REQUEST_HEADERS:Host -> 192.0.2.1",
                                                user_agent: "python-requests/2.31.0",
                                                tags: ["OWASP_CRS/PROTOCOL_VIOLATION"],
                                              },
                                            ];

                                            const mockDisabledRules = [
                                              {
                                                rule_id: 920350,
                                                domain: "example.com",
                                                description: "Host header is a numeric IP address",
                                                disabled_at: "2026-11-12 09:00:00",
                                                reason: "Internal microservice webhook traffic using IP addresses",
                                              },
                                            ];

                                            const mockWafPolicies: Record<string, any> = {
                                              "example.com": {
                                                domain: "example.com",
                                                mode: "block",
                                                disabled_rules: [920350],
                                                paranoia_level: 1,
                                                updated_at: "2026-11-12 09:00:00",
                                              },
                                              "all": {
                                                domain: "all",
                                                mode: "block",
                                                disabled_rules: [],
                                                paranoia_level: 1,
                                                updated_at: "2026-11-12 09:00:00",
                                              },
                                            };

                                            const computeStats = (dom?: string) => {
                                              const list = dom && dom !== "all" ? mockWafEvents.filter((e) => e.domain === dom) : mockWafEvents;
                                              return {
                                                total_events: list.length,
                                                blocked_events: list.filter((e) => e.action === "blocked").length,
                                                detected_events: list.filter((e) => e.action === "detected").length,
                                                sqli_count: list.filter((e) => e.category === "sqli").length,
                                                xss_count: list.filter((e) => e.category === "xss").length,
                                                rce_count: list.filter((e) => e.category === "rce").length,
                                                scanner_count: list.filter((e) => e.category === "scanner").length,
                                              };
                                            };

                                            return [
                                              // GET /api/v1/client/waf/events: what the firewall refused, in the real shape.
                                              http.get(/\/api\/v1\/client\/waf\/events$/, () => {
                                                return HttpResponse.json({
                                                  range: "24h",
                                                  available: true,
                                                  partial: false,
                                                  items: mockWafEvents.filter((e) => e.action === "blocked").map((e) => ({
                                                    at_ms: Date.parse(e.timestamp) || Date.now(),
                                                    method: e.http_method,
                                                    host: e.domain,
                                                    uri: e.uri,
                                                    rule_id: e.rule_id,
                                                    message: e.rule_message,
                                                    blocked: true,
                                                  })),
                                                });
                                              }),

                                              // GET /api/v1/client/waf/rules/disabled (TODO(api:T11.04))
                                              http.get(/\/api\/v1\/client\/waf\/rules\/disabled$/, ({ request }) => {
                                                const url = new URL(request.url);
                                                const domain = url.searchParams.get("domain");
                                                if (domain && domain !== "all") {
                                                  return HttpResponse.json(mockDisabledRules.filter((r) => r.domain === domain));
                                                }
                                                return HttpResponse.json(mockDisabledRules);
                                              }),

                                              // GET /api/v1/client/waf/policy (TODO(api:T11.04))
                                              http.get(/\/api\/v1\/client\/waf\/policy$/, ({ request }) => {
                                                const url = new URL(request.url);
                                                const domain = url.searchParams.get("domain") || "example.com";
                                                return HttpResponse.json(mockWafPolicies[domain] || {
                                                  domain,
                                                  mode: "block",
                                                  disabled_rules: [],
                                                  paranoia_level: 1,
                                                  updated_at: new Date().toISOString(),
                                                });
                                              }),

                                              // GET /api/v1/client/waf/stats (TODO(api:T11.04))
                                              http.get(/\/api\/v1\/client\/waf\/stats$/, ({ request }) => {
                                                const url = new URL(request.url);
                                                const domain = url.searchParams.get("domain") || "example.com";
                                                return HttpResponse.json(computeStats(domain));
                                              }),

                                              // POST /api/v1/client/waf/rules/disable (TODO(api:T11.04))
                                              http.post(/\/api\/v1\/client\/waf\/rules\/disable$/, async ({ request }) => {
                                                const body = (await request.json().catch(() => ({}))) as any;
                                                const ruleId = Number(body.rule_id);
                                                const domain = body.domain || "example.com";
                                                const existingIdx = mockDisabledRules.findIndex((r) => r.rule_id === ruleId && r.domain === domain);
                                                if (existingIdx === -1) {
                                                  mockDisabledRules.unshift({
                                                    rule_id: ruleId,
                                                    domain,
                                                    description: body.reason || `Rule ${ruleId} disabled for false positive`,
                                                    disabled_at: new Date().toISOString().slice(0, 19).replace("T", " "),
                                                    reason: body.reason || "False positive report via UI",
                                                  });
                                                }
                                                return HttpResponse.json({ success: true, rule_id: ruleId, domain });
                                              }),

                                              // POST /api/v1/client/waf/rules/enable (TODO(api:T11.04))
                                              http.post(/\/api\/v1\/client\/waf\/rules\/enable$/, async ({ request }) => {
                                                const body = (await request.json().catch(() => ({}))) as any;
                                                const ruleId = Number(body.rule_id);
                                                const domain = body.domain || "example.com";
                                                const idx = mockDisabledRules.findIndex((r) => r.rule_id === ruleId && r.domain === domain);
                                                if (idx !== -1) {
                                                  mockDisabledRules.splice(idx, 1);
                                                }
                                                return HttpResponse.json({ success: true, rule_id: ruleId, domain });
                                              }),

                                              // POST /api/v1/client/waf/policy (TODO(api:T11.04))
                                              http.post(/\/api\/v1\/client\/waf\/policy$/, async ({ request }) => {
                                                const body = (await request.json().catch(() => ({}))) as any;
                                                const domain = body.domain || "example.com";
                                                if (!mockWafPolicies[domain]) {
                                                  mockWafPolicies[domain] = { domain, mode: "block", disabled_rules: [], paranoia_level: 1, updated_at: "" };
                                                }
                                                mockWafPolicies[domain].mode = body.mode || "block";
                                                mockWafPolicies[domain].updated_at = new Date().toISOString();
                                                return HttpResponse.json({ success: true, policy: mockWafPolicies[domain] });
                                              }),

                                              // GET /api/v1/admin/waf/policy (TODO(api:T11.04))
                                              http.get(/\/api\/v1\/admin\/waf\/policy$/, ({ request }) => {
                                                const url = new URL(request.url);
                                                const domain = url.searchParams.get("domain") || "all";
                                                return HttpResponse.json(mockWafPolicies[domain] || {
                                                  domain,
                                                  mode: "block",
                                                  disabled_rules: [],
                                                  paranoia_level: 1,
                                                  updated_at: new Date().toISOString(),
                                                });
                                              }),

                                              // POST /api/v1/admin/waf/policy (TODO(api:T11.04))
                                              http.post(/\/api\/v1\/admin\/waf\/policy$/, async ({ request }) => {
                                                const body = (await request.json().catch(() => ({}))) as any;
                                                const domain = body.domain || "all";
                                                if (!mockWafPolicies[domain]) {
                                                  mockWafPolicies[domain] = { domain, mode: "block", disabled_rules: [], paranoia_level: 1, updated_at: "" };
                                                }
                                                mockWafPolicies[domain].mode = body.mode || "block";
                                                return HttpResponse.json({ success: true, policy: mockWafPolicies[domain] });
                                              }),
...(() => {
                                                const mockIpBlocks = new Map<string, Array<{ id: string; cidr: string; note: string | null; created_at: number }>>();

                                                const mockAdminFirewallOverview: {
                                                  node_id: string;
                                                  status: "active" | "inactive";
                                                  nftables_version: string;
                                                  active_ports: Array<{ id: string; port: number; protocol: "tcp" | "udp"; service_name: string; is_system_protected: boolean; status: "open" | "closed" }>;
                                                  active_bans: Array<{ id: string; ip: string; reason: string; source: "bruteforce_auto" | "admin_manual"; service: string; banned_at: string; expires_at: string | null }>;
                                                  rate_limit_enabled: boolean;
                                                  rate_limit_rps: number;
                                                } = {
                                                  node_id: "node-1",
                                                  status: "active" as const,
                                                  nftables_version: "nftables v1.0.9 (Kernel Packet Filter)",
                                                  active_ports: [
                                                    { id: "port-1", port: 22, protocol: "tcp" as const, service_name: "SSH Management", is_system_protected: true, status: "open" as const },
                                                    { id: "port-2", port: 80, protocol: "tcp" as const, service_name: "HTTP Web Traffic", is_system_protected: true, status: "open" as const },
                                                    { id: "port-3", port: 443, protocol: "tcp" as const, service_name: "HTTPS TLS Traffic", is_system_protected: true, status: "open" as const },
                                                    { id: "port-4", port: 21, protocol: "tcp" as const, service_name: "FTP Control", is_system_protected: false, status: "open" as const },
                                                    { id: "port-5", port: 8080, protocol: "tcp" as const, service_name: "Dev Webhook Port", is_system_protected: false, status: "open" as const },
                                                  ],
                                                  active_bans: [
                                                    {
                                                      id: "ban-1",
                                                      ip: "198.51.100.22",
                                                      reason: "Exceeded 10 failed panel login attempts",
                                                      source: "bruteforce_auto" as const,
                                                      service: "panel_login" as const,
                                                      banned_at: "2026-11-13 10:15:00",
                                                      expires_at: "2026-11-13 11:15:00",
                                                    },
                                                    {
                                                      id: "ban-2",
                                                      ip: "203.0.113.99",
                                                      reason: "Repeated SFTP dictionary attacks",
                                                      source: "bruteforce_auto" as const,
                                                      service: "sftp" as const,
                                                      banned_at: "2026-11-13 10:45:00",
                                                      expires_at: "2026-11-13 11:45:00",
                                                    },
                                                  ],
                                                  rate_limit_enabled: true,
                                                  rate_limit_rps: 50,
                                                };

                                                return [
                                                  // The addresses one domain turns away.
                                                  http.get(/\/api\/v1\/client\/waf\/domains\/([^/]+)\/ip-blocks$/, ({ params }) => {
                                                    const list = mockIpBlocks.get(String(params[0])) ?? [];
                                                    return HttpResponse.json({ items: list, limit: 100 });
                                                  }),

                                                  http.post(/\/api\/v1\/client\/waf\/domains\/([^/]+)\/ip-blocks$/, async ({ request, params }) => {
                                                    const body = (await request.json().catch(() => ({}))) as { cidr?: string; note?: string | null };
                                                    const domain = String(params[0]);
                                                    const list = mockIpBlocks.get(domain) ?? [];
                                                    if (!body.cidr || !/^[0-9a-fA-F:.]+(\/\d{1,3})?$/.test(body.cidr)) {
                                                      return problem(422, "validation_failed", "Not an IP address or network.");
                                                    }
                                                    if (list.some((b) => b.cidr === body.cidr)) {
                                                      return problem(409, "ip_block_exists", "That address is already turned away.");
                                                    }
                                                    const added = { id: `ipb-${Date.now()}-${list.length}`, cidr: body.cidr, note: body.note ?? null, created_at: Math.floor(Date.now() / 1000) };
                                                    mockIpBlocks.set(domain, [...list, added]);
                                                    return HttpResponse.json(added, { status: 201 });
                                                  }),

                                                  http.delete(/\/api\/v1\/client\/waf\/domains\/([^/]+)\/ip-blocks\/([^/]+)$/, ({ params }) => {
                                                    const domain = String(params[0]);
                                                    const list = mockIpBlocks.get(domain) ?? [];
                                                    mockIpBlocks.set(domain, list.filter((b) => b.id !== String(params[1])));
                                                    return new HttpResponse(null, { status: 204 });
                                                  }),

                                                   ...(() => {
                                                     const mockSecurityOverview = {
                                                       stats: {
                                                         score: 78,
                                                         grade: "B" as const,
                                                         critical_count: 2,
                                                         warning_count: 4,
                                                         info_count: 0,
                                                         resolved_count: 4,
                                                         login_failures_24h: 14,
                                                         waf_blocks_24h: 8,
                                                         malware_detected: 1,
                                                         admins_without_2fa: 2,
                                                         expiring_ssl_certs: 1,
                                                         outdated_images: 3,
                                                       },
                                                       issues: [
                                                         {
                                                           id: "sec-issue-auth-failures",
                                                           category: "auth" as const,
                                                           severity: "critical" as const,
                                                           title: "Brute-Force SSH/Panel Login Attempts Detected",
                                                           title_bn: "ব্রুট-ফোর্স SSH ও প্যানেল লগইন চেষ্টা শনাক্ত হয়েছে",
                                                           description: "14 consecutive failed login attempts detected from IP 203.0.113.88 targeting admin accounts.",
                                                           description_bn: "IP 203.0.113.88 থেকে অ্যাডমিন অ্যাকাউন্টে ১৪ বার ধারাবাহিক ব্যর্থ লগইন চেষ্টা শনাক্ত হয়েছে।",
                                                           timestamp: new Date(Date.now() - 15 * 60 * 1000).toISOString(),
                                                           resource: "203.0.113.88 (fail2ban/panel-auth)",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixBanIp",
                                                             default_label: "Ban IP in Firewall",
                                                             type: "modal_action" as const,
                                                             action_id: "ban_ip",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                         {
                                                           id: "sec-issue-malware-eicar",
                                                           category: "malware" as const,
                                                           severity: "critical" as const,
                                                           title: "Suspicious Webshell / Malware Detected",
                                                           title_bn: "সন্দেহভাজন ওয়েবশেল বা ম্যালওয়্যার শনাক্ত হয়েছে",
                                                           description: "ClamAV scan flagged /var/www/site1/public/backdoor.php as PHP.Trojan.WebShell-1.",
                                                           description_bn: "ClamAV স্ক্যানে /var/www/site1/public/backdoor.php ফাইলটি ম্যালওয়্যার হিসেবে ধরা পড়েছে।",
                                                           timestamp: new Date(Date.now() - 45 * 60 * 1000).toISOString(),
                                                           resource: "/var/www/site1/public/backdoor.php",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixQuarantineFile",
                                                             default_label: "Quarantine Malware File",
                                                             type: "modal_action" as const,
                                                             action_id: "quarantine_file",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                         {
                                                           id: "sec-issue-waf-attacks",
                                                           category: "waf" as const,
                                                           severity: "warning" as const,
                                                           title: "Coraza WAF Triggered on Domain client-store.com",
                                                           title_bn: "ক্লায়েন্ট ডোমেইনে Coraza WAF ব্লকিং ট্রিগার হয়েছে",
                                                           description: "Multiple SQL Injection (OWASP-942100) payloads detected on /api/search endpoint.",
                                                           description_bn: "/api/search এন্ডপয়েন্টে একাধিক SQL ইনজেকশন পে-লোড শনাক্ত ও ব্লক করা হয়েছে।",
                                                           timestamp: new Date(Date.now() - 2 * 3600 * 1000).toISOString(),
                                                           resource: "client-store.com (WAF Core Ruleset)",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixReviewWaf",
                                                             default_label: "Review WAF Events",
                                                             type: "link" as const,
                                                             target_url: "/admin/waf",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                         {
                                                           id: "sec-issue-admins-no-2fa",
                                                           category: "auth" as const,
                                                           severity: "warning" as const,
                                                           title: "Admin Accounts Missing Two-Factor Authentication",
                                                           title_bn: "অ্যাডমিন অ্যাকাউন্টে টু-ফ্যাক্টর অথেনটিকেশন (2FA) চালু নেই",
                                                           description: "2 superadmin/operator accounts have not enrolled TOTP or WebAuthn hardware keys.",
                                                           description_bn: "২ জন অ্যাডমিনের অ্যাকাউন্টে কোনো TOTP বা নিরাপত্তা চাবি কনফিগার করা নেই।",
                                                           timestamp: new Date(Date.now() - 12 * 3600 * 1000).toISOString(),
                                                           resource: "admin_user_2, ops_admin",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixEnforce2fa",
                                                             default_label: "Enforce 2FA on Admins",
                                                             type: "modal_action" as const,
                                                             action_id: "enforce_2fa",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                         {
                                                           id: "sec-issue-ssl-expiring",
                                                           category: "ssl" as const,
                                                           severity: "warning" as const,
                                                           title: "SSL/TLS Certificate Expiring Soon",
                                                           title_bn: "SSL/TLS সনদের মেয়াদ দ্রুত শেষ হতে চলেছে",
                                                           description: "Let's Encrypt certificate for billing.example.com will expire in 3 days.",
                                                           description_bn: "billing.example.com ডোমেইনের SSL সার্টিফিকেটের মেয়াদ আগামী ৩ দিনের মধ্যে শেষ হবে।",
                                                           timestamp: new Date(Date.now() - 18 * 3600 * 1000).toISOString(),
                                                           resource: "billing.example.com",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixRenewSsl",
                                                             default_label: "Renew SSL Certificate",
                                                             type: "link" as const,
                                                             target_url: "/ssl",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                         {
                                                           id: "sec-issue-outdated-image",
                                                           category: "container" as const,
                                                           severity: "warning" as const,
                                                           title: "Outdated Base Docker Image with Known CVEs",
                                                           title_bn: "পরিচিত নিরাপত্তা ত্রুটিসহ পুরনো ডকার ইমেজ শনাক্ত",
                                                           description: "Container image php:8.2-fpm has 4 high-severity vulnerabilities in libxml2 and curl.",
                                                           description_bn: "php:8.2-fpm ইমেজে libxml2 ও curl-এ ৪টি হাই-সিভিয়ারিটি CVE পাওয়া গেছে।",
                                                           timestamp: new Date(Date.now() - 24 * 3600 * 1000).toISOString(),
                                                           resource: "docker.io/library/php:8.2-fpm",
                                                           fix_action: {
                                                             label_key: "securityCenter.fixPullImage",
                                                             default_label: "Update Container Image",
                                                             type: "modal_action" as const,
                                                             action_id: "update_image",
                                                           },
                                                           status: "open" as "open" | "resolved",
                                                         },
                                                       ],
                                                     };

                                                     return [
                                                       // GET /api/v1/admin/security/overview (TODO(api:T11.11))
                                                       http.get(/\/api\/v1\/admin\/security\/overview$/, () => {
                                                         return HttpResponse.json(mockSecurityOverview);
                                                       }),

                                                       // POST /api/v1/admin/security/scan (TODO(api:T11.11))
                                                       http.post(/\/api\/v1\/admin\/security\/scan$/, () => {
                                                         return HttpResponse.json(mockSecurityOverview);
                                                       }),

                                                       // POST /api/v1/admin/security/issues/:id/fix (TODO(api:T11.11))
                                                       http.post(/\/api\/v1\/admin\/security\/issues\/([^/]+)\/fix$/, ({ params }) => {
                                                         const id = String(params[0] || params.id);
                                                         const issue = mockSecurityOverview.issues.find((i) => i.id === id);
                                                         if (issue) {
                                                           issue.status = "resolved";
                                                           mockSecurityOverview.stats.resolved_count += 1;
                                                           if (issue.severity === "critical") {
                                                             mockSecurityOverview.stats.critical_count = Math.max(0, mockSecurityOverview.stats.critical_count - 1);
                                                           } else if (issue.severity === "warning") {
                                                             mockSecurityOverview.stats.warning_count = Math.max(0, mockSecurityOverview.stats.warning_count - 1);
                                                           }
                                                           mockSecurityOverview.stats.score = Math.min(100, mockSecurityOverview.stats.score + 5);
                                                         }
                                                         return HttpResponse.json({ success: true, issue });
                                                       }),
                                                     ];
                                                   })(),

                                                   ...(() => {
                                                     // Applications, shaped like `CatalogEntry`, `AppView` and the WordPress state in openapi.json
                                                     const catalog = [
                                                       { slug: "wordpress", name: "WordPress", category: "cms", version: "6.7.1", description: "World's most popular web publishing platform and CMS.", document_root: "", needs_database: true, needs_admin: true },
                                                       { slug: "joomla", name: "Joomla", category: "cms", version: "5.2.1", description: "Powerful open-source CMS for corporate websites and portals.", document_root: "", needs_database: true, needs_admin: true },
                                                       { slug: "laravel", name: "Laravel Skeleton", category: "framework", version: "11.31.0", description: "The PHP framework for web artisans.", document_root: "public", needs_database: true, needs_admin: false },
                                                       { slug: "static", name: "Static HTML5 Starter", category: "static", version: "1.0.0", description: "A fast responsive HTML5/CSS3 starter.", document_root: "", needs_database: false, needs_admin: false },
                                                     ];
                                                     let apps = [
                                                       { id: "app_wp_01", kind: "wordpress", name: "My Awesome Site", path: "/", fqdn: "alpha-hosting.com", domain_id: "dom_01", database_id: "db_01" as string | null, version: "6.7.1" as string | null, status: "active", error: null as string | null, created_at: Math.floor(Date.now() / 1000) - 86400 * 5 },
                                                       { id: "app_joomla_01", kind: "joomla", name: "Company Portal", path: "/portal", fqdn: "alpha-hosting.com", domain_id: "dom_01", database_id: "db_02", version: "5.2.1", status: "active", error: null as string | null, created_at: Math.floor(Date.now() / 1000) - 86400 * 2 },
                                                     ];
                                                     let maintenance = false;
                                                     let plugins = [
                                                       { name: "woocommerce", status: "active", version: "8.5.1", update_available: true, update_version: "8.6.0" },
                                                       { name: "wordpress-seo", status: "active", version: "22.0", update_available: false, update_version: null as string | null },
                                                     ];
                                                     let scan: Record<string, unknown> | null = null;
                                                     const overview = () => ({
                                                       state: "done",
                                                       overview: {
                                                         core: "6.7.1",
                                                         core_update: null,
                                                         maintenance,
                                                         plugins,
                                                         themes: [{ name: "twentytwentyfour", status: "active", version: "1.0", update_available: false, update_version: null }],
                                                       },
                                                       scan,
                                                     });
                                                     const makeApp = async (request: Request) => {
                                                       const body = (await request.json()) as { slug?: string; domain_id: string; path?: string; title: string };
                                                       const kind = body.slug ?? "wordpress";
                                                       const created = {
                                                         id: `app_${Date.now()}`,
                                                         kind,
                                                         name: body.title,
                                                         path: body.path ?? "/",
                                                         fqdn: "alpha-hosting.com",
                                                         domain_id: body.domain_id,
                                                         database_id: null,
                                                         version: catalog.find((c) => c.slug === kind)?.version ?? null,
                                                         status: "installing",
                                                         error: null,
                                                         created_at: Math.floor(Date.now() / 1000),
                                                       };
                                                       apps = [created, ...apps];
                                                       return HttpResponse.json(created, { status: 202 });
                                                     };
                                                     return [
                                                       http.get(/\/api\/v1\/client\/apps\/catalog$/, () => HttpResponse.json({ items: catalog })),
                                                       http.get(/\/api\/v1\/client\/apps$/, () => HttpResponse.json({ items: apps })),
                                                       http.post(/\/api\/v1\/client\/apps\/install$/, ({ request }) => makeApp(request)),
                                                       http.post(/\/api\/v1\/client\/apps\/wordpress$/, ({ request }) => makeApp(request)),
                                                       http.get(/\/api\/v1\/client\/apps\/([^/]+)\/wordpress$/, () => HttpResponse.json(overview())),
                                                       http.get(/\/api\/v1\/client\/apps\/([^/]+)\/wordpress\/scan$/, () => {
                                                         scan = { checksums_ok: true, config_mode: "0640", debug: false, extra: [], file_edit_allowed: true, missing: [], modified: [], world_writable: 0, xmlrpc_disabled: false };
                                                         return HttpResponse.json({ state: "done", scan });
                                                       }),
                                                       http.post(/\/api\/v1\/client\/apps\/([^/]+)\/wordpress\/maintenance$/, async ({ request }) => {
                                                         maintenance = ((await request.json()) as { on: boolean }).on;
                                                         return HttpResponse.json({ state: "done" });
                                                       }),
                                                       http.post(/\/api\/v1\/client\/apps\/([^/]+)\/wordpress\/plugins\/update$/, async ({ request }) => {
                                                         const { slug } = (await request.json()) as { slug?: string | null };
                                                         plugins = plugins.map((p) => (!slug || p.name === slug ? { ...p, version: p.update_version ?? p.version, update_available: false } : p));
                                                         return HttpResponse.json({ state: "done" });
                                                       }),
                                                       http.post(/\/api\/v1\/client\/apps\/([^/]+)\/wordpress\/harden$/, () => {
                                                         scan = { checksums_ok: true, config_mode: "0640", debug: false, extra: [], file_edit_allowed: false, missing: [], modified: [], world_writable: 0, xmlrpc_disabled: true };
                                                         return HttpResponse.json({ state: "done" });
                                                       }),
...(() => {
                                                         // Email Handlers (T13.12, T13.05, T13.06, T13.10, T13.11)
                                                         let mockMailboxes = [
                                                           {
                                                             id: "mb-1",
                                                             email: "info@example.com",
                                                             username: "info",
                                                             domain: "example.com",
                                                             quota_mb: 2048,
                                                             used_bytes: 524288000,
                                                             created_at: new Date(Date.now() - 30 * 86400000).toISOString(),
                                                             webmail_url: "https://webmail.example.com",
                                                           },
                                                           {
                                                             id: "mb-2",
                                                             email: "support@example.com",
                                                             username: "support",
                                                             domain: "example.com",
                                                             quota_mb: 5120,
                                                             used_bytes: 4294967296,
                                                             created_at: new Date(Date.now() - 60 * 86400000).toISOString(),
                                                             webmail_url: "https://webmail.example.com",
                                                           },
                                                           {
                                                             id: "mb-3",
                                                             email: "admin@mysite.org",
                                                             username: "admin",
                                                             domain: "mysite.org",
                                                             quota_mb: 1024,
                                                             used_bytes: 104857600,
                                                             created_at: new Date(Date.now() - 10 * 86400000).toISOString(),
                                                             webmail_url: "https://webmail.mysite.org",
                                                           },
                                                         ];

                                                         let mockForwarders = [
                                                           {
                                                             id: "fwd-1",
                                                             source: "sales@example.com",
                                                             destination: "sales-team@external-crm.com",
                                                             domain: "example.com",
                                                             created_at: new Date(Date.now() - 15 * 86400000).toISOString(),
                                                           },
                                                           {
                                                             id: "fwd-2",
                                                             source: "billing@example.com",
                                                             destination: "accounting@finance-corp.com",
                                                             domain: "example.com",
                                                             created_at: new Date(Date.now() - 25 * 86400000).toISOString(),
                                                           },
                                                         ];

                                                         let mockAutoresponders = [
                                                           {
                                                             id: "ar-1",
                                                             email: "support@example.com",
                                                             domain: "example.com",
                                                             subject: "We received your ticket",
                                                             body: "Thank you for contacting support. Our team will review your inquiry within 24 hours.",
                                                             is_active: true,
                                                           },
                                                         ];

                                                         let mockMailQueue = [
                                                           {
                                                             id: "4YZ8Q91024B",
                                                             sender: "notifications@example.com",
                                                             recipient: "user123@remote-domain.com",
                                                             status: "deferred" as "queued" | "deferred" | "bounced",
                                                             error_message: "Connection timed out (port 25)",
                                                             queued_at: new Date(Date.now() - 1200000).toISOString(),
                                                             retries: 3,
                                                             size_bytes: 45020,
                                                           },
                                                           {
                                                             id: "4YZ8T58291A",
                                                             sender: "info@example.com",
                                                             recipient: "client@gmail.com",
                                                             status: "queued" as "queued" | "deferred" | "bounced",
                                                             queued_at: new Date(Date.now() - 60000).toISOString(),
                                                             retries: 0,
                                                             size_bytes: 12850,
                                                           },
                                                         ];

                                                         const mockDeliverability = [
                                                           {
                                                             domain: "example.com",
                                                             spf_status: "pass" as const,
                                                             dkim_status: "pass" as const,
                                                             dmarc_status: "pass" as const,
                                                             mx_status: "pass" as const,
                                                             rbl_blacklisted: false,
                                                             rbl_listings: [],
                                                             recommendations: [
                                                               "DKIM selector builtpanel is active with 2048-bit RSA key.",
                                                               "DMARC policy is currently set to p=quarantine.",
                                                             ],
                                                           },
                                                           {
                                                             domain: "mysite.org",
                                                             spf_status: "pass" as const,
                                                             dkim_status: "fail" as const,
                                                             dmarc_status: "missing" as const,
                                                             mx_status: "pass" as const,
                                                             rbl_blacklisted: false,
                                                             rbl_listings: [],
                                                             recommendations: [
                                                               "Add a TXT record for _dmarc.mysite.org with v=DMARC1; p=none;.",
                                                               "Sign outgoing mail with DKIM key to improve delivery.",
                                                             ],
                                                           },
                                                         ];

                                                         return [
                                                           // GET /api/v1/email/mailboxes (TODO(api:T13.05))
                                                           http.get(/\/api\/v1\/email\/mailboxes$/, ({ request }) => {
                                                             const url = new URL(request.url);
                                                             const domain = url.searchParams.get("domain");
                                                             const filtered = domain
                                                               ? mockMailboxes.filter((m) => m.domain === domain)
                                                               : mockMailboxes;
                                                             return HttpResponse.json(filtered);
                                                           }),

                                                           // POST /api/v1/email/mailboxes (TODO(api:T13.05))
                                                           http.post(/\/api\/v1\/email\/mailboxes$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newMailbox = {
                                                               id: `mb-${Date.now()}`,
                                                               email: `${body.username}@${body.domain}`,
                                                               username: body.username,
                                                               domain: body.domain,
                                                               quota_mb: Number(body.quota_mb) || 1024,
                                                               used_bytes: 0,
                                                               created_at: new Date().toISOString(),
                                                               webmail_url: `https://webmail.${body.domain}`,
                                                             };
                                                             mockMailboxes.push(newMailbox);
                                                             return HttpResponse.json(newMailbox, { status: 201 });
                                                           }),

                                                           // DELETE /api/v1/email/mailboxes/:id (TODO(api:T13.05))
                                                           http.delete(/\/api\/v1\/email\/mailboxes\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockMailboxes = mockMailboxes.filter((m) => m.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // POST /api/v1/email/mailboxes/:id/password (TODO(api:T13.05))
                                                           http.post(/\/api\/v1\/email\/mailboxes\/([^/]+)\/password$/, () => {
                                                             return HttpResponse.json({ success: true });
                                                           }),

                                                           // GET /api/v1/email/forwarders (TODO(api:T13.06))
                                                           http.get(/\/api\/v1\/email\/forwarders$/, ({ request }) => {
                                                             const url = new URL(request.url);
                                                             const domain = url.searchParams.get("domain");
                                                             const filtered = domain
                                                               ? mockForwarders.filter((f) => f.domain === domain)
                                                               : mockForwarders;
                                                             return HttpResponse.json(filtered);
                                                           }),

                                                           // POST /api/v1/email/forwarders (TODO(api:T13.06))
                                                           http.post(/\/api\/v1\/email\/forwarders$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newFwd = {
                                                               id: `fwd-${Date.now()}`,
                                                               source: body.source,
                                                               destination: body.destination,
                                                               domain: body.domain,
                                                               created_at: new Date().toISOString(),
                                                             };
                                                             mockForwarders.push(newFwd);
                                                             return HttpResponse.json(newFwd, { status: 201 });
                                                           }),

                                                           // DELETE /api/v1/email/forwarders/:id (TODO(api:T13.06))
                                                           http.delete(/\/api\/v1\/email\/forwarders\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockForwarders = mockForwarders.filter((f) => f.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/email/autoresponders (TODO(api:T13.06))
                                                           http.get(/\/api\/v1\/email\/autoresponders$/, ({ request }) => {
                                                             const url = new URL(request.url);
                                                             const domain = url.searchParams.get("domain");
                                                             const filtered = domain
                                                               ? mockAutoresponders.filter((a) => a.domain === domain)
                                                               : mockAutoresponders;
                                                             return HttpResponse.json(filtered);
                                                           }),

                                                           // POST /api/v1/email/autoresponders (TODO(api:T13.06))
                                                           http.post(/\/api\/v1\/email\/autoresponders$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newAr = {
                                                               id: `ar-${Date.now()}`,
                                                               email: body.email,
                                                               domain: body.domain,
                                                               subject: body.subject,
                                                               body: body.body,
                                                               is_active: true,
                                                               start_date: body.start_date,
                                                               end_date: body.end_date,
                                                             };
                                                             mockAutoresponders.push(newAr);
                                                             return HttpResponse.json(newAr, { status: 201 });
                                                           }),

                                                           // DELETE /api/v1/email/autoresponders/:id (TODO(api:T13.06))
                                                           http.delete(/\/api\/v1\/email\/autoresponders\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockAutoresponders = mockAutoresponders.filter((a) => a.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/email/config (TODO(api:T13.12))
                                                           http.get(/\/api\/v1\/email\/config$/, ({ request }) => {
                                                             const url = new URL(request.url);
                                                             const domain = url.searchParams.get("domain") || "example.com";
                                                             return HttpResponse.json({
                                                               incoming_server: `mail.${domain}`,
                                                               incoming_imap_port: 993,
                                                               incoming_pop3_port: 995,
                                                               incoming_security: "SSL/TLS",
                                                               outgoing_server: `mail.${domain}`,
                                                               outgoing_smtp_port: 465,
                                                               outgoing_submission_port: 587,
                                                               outgoing_security: "SSL/TLS (or STARTTLS)",
                                                               auth_required: true,
                                                             });
                                                           }),

                                                           // GET /api/v1/admin/email/queue (TODO(api:T13.10))
                                                           http.get(/\/api\/v1\/admin\/email\/queue$/, () => {
                                                             return HttpResponse.json(mockMailQueue);
                                                           }),

                                                           // POST /api/v1/admin/email/queue/flush (TODO(api:T13.10))
                                                           http.post(/\/api\/v1\/admin\/email\/queue\/flush$/, () => {
                                                             mockMailQueue = [];
                                                             return HttpResponse.json({ success: true, count: 2 });
                                                           }),

                                                           // DELETE /api/v1/admin/email/queue/:id (TODO(api:T13.10))
                                                           http.delete(/\/api\/v1\/admin\/email\/queue\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockMailQueue = mockMailQueue.filter((q) => q.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/admin/email/deliverability (TODO(api:T13.11))
                                                           http.get(/\/api\/v1\/admin\/email\/deliverability$/, () => {
                                                             return HttpResponse.json(mockDeliverability);
                                                           }),
...(() => {
                                                         // Reseller Handlers (T14.01, T14.02, T14.03, T14.04, T14.05, T14.07)
                                                         let mockResellerPackages = [
                                                           {
                                                             id: "pkg-1",
                                                             name: "Growth Agency Tier",
                                                             max_accounts: 25,
                                                             max_disk_gb: 150,
                                                             max_bandwidth_gb: 1500,
                                                             allow_overselling: true,
                                                             allowed_plans: ["all"],
                                                             created_at: new Date(Date.now() - 60 * 86400000).toISOString(),
                                                           },
                                                           {
                                                             id: "pkg-2",
                                                             name: "Enterprise Partner Tier",
                                                             max_accounts: 100,
                                                             max_disk_gb: 500,
                                                             max_bandwidth_gb: 5000,
                                                             allow_overselling: false,
                                                             allowed_plans: ["all"],
                                                             created_at: new Date(Date.now() - 90 * 86400000).toISOString(),
                                                           },
                                                         ];

                                                         const mockResellerAccount = {
                                                           id: "reseller-1",
                                                           username: "apex_reseller",
                                                           email: "partner@apexcloud.net",
                                                           company_name: "Apex Cloud Solutions",
                                                           package_id: "pkg-1",
                                                           package_name: "Growth Agency Tier",
                                                           status: "active" as "active" | "suspended",
                                                           accounts_count: 8,
                                                           disk_used_gb: 42,
                                                           bandwidth_used_gb: 380,
                                                           branding: {
                                                             brand_name: "Apex Cloud Solutions",
                                                             logo_url: "https://example.com/logo.png",
                                                             primary_color: "#2563eb",
                                                             support_email: "support@apexcloud.net",
                                                             custom_login_message: "Welcome to your high performance cloud control panel.",
                                                           },
                                                           created_at: new Date(Date.now() - 45 * 86400000).toISOString(),
                                                         };

                                                         let mockAdminResellers = [
                                                           mockResellerAccount,
                                                           {
                                                             id: "reseller-2",
                                                             username: "pixel_craft",
                                                             email: "admin@pixelcraft.agency",
                                                             company_name: "PixelCraft Hosting",
                                                             package_id: "pkg-2",
                                                             package_name: "Enterprise Partner Tier",
                                                             status: "active" as "active" | "suspended",
                                                             accounts_count: 34,
                                                             disk_used_gb: 180,
                                                             bandwidth_used_gb: 1200,
                                                             branding: {
                                                               brand_name: "PixelCraft Hosting",
                                                               logo_url: "https://example.com/pixel-logo.png",
                                                               primary_color: "#7c3aed",
                                                               support_email: "help@pixelcraft.agency",
                                                               custom_login_message: "Powered by PixelCraft Studio Cloud.",
                                                             },
                                                             created_at: new Date(Date.now() - 80 * 86400000).toISOString(),
                                                           },
                                                         ];

                                                         let mockResellerPlans = [
                                                           {
                                                             id: "rp-1",
                                                             reseller_id: "reseller-1",
                                                             name: "Client Starter",
                                                             disk_mb: 5120,
                                                             bandwidth_mb: 51200,
                                                             domains_limit: 1,
                                                             databases_limit: 2,
                                                             mailboxes_limit: 5,
                                                             created_at: new Date(Date.now() - 30 * 86400000).toISOString(),
                                                           },
                                                           {
                                                             id: "rp-2",
                                                             reseller_id: "reseller-1",
                                                             name: "Business Pro",
                                                             disk_mb: 15360,
                                                             bandwidth_mb: 153600,
                                                             domains_limit: 5,
                                                             databases_limit: 10,
                                                             mailboxes_limit: 20,
                                                             created_at: new Date(Date.now() - 20 * 86400000).toISOString(),
                                                           },
                                                         ];

                                                         let mockResellerUsers = [
                                                           {
                                                             id: "ru-1",
                                                             reseller_id: "reseller-1",
                                                             username: "novatech",
                                                             email: "admin@novatech.io",
                                                             domain: "novatech.io",
                                                             plan_name: "Business Pro",
                                                             disk_used_mb: 4200,
                                                             status: "active" as "active" | "suspended",
                                                             created_at: new Date(Date.now() - 15 * 86400000).toISOString(),
                                                           },
                                                           {
                                                             id: "ru-2",
                                                             reseller_id: "reseller-1",
                                                             username: "sweetbakery",
                                                             email: "contact@sweetbakery.com",
                                                             domain: "sweetbakery.com",
                                                             plan_name: "Client Starter",
                                                             disk_used_mb: 1200,
                                                             status: "active" as "active" | "suspended",
                                                             created_at: new Date(Date.now() - 10 * 86400000).toISOString(),
                                                           },
                                                         ];

                                                         return [
                                                           // GET /api/v1/reseller/me (TODO(api:T14.01))
                                                           http.get(/\/api\/v1\/reseller\/me$/, () => {
                                                             return HttpResponse.json(mockResellerAccount);
                                                           }),

                                                           // GET /api/v1/reseller/me/package (TODO(api:T14.01))
                                                           http.get(/\/api\/v1\/reseller\/me\/package$/, () => {
                                                             const pkg = mockResellerPackages.find((p) => p.id === mockResellerAccount.package_id) || mockResellerPackages[0];
                                                             return HttpResponse.json(pkg);
                                                           }),

                                                           // PUT /api/v1/reseller/me/branding (TODO(api:T14.05))
                                                           http.put(/\/api\/v1\/reseller\/me\/branding$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             mockResellerAccount.branding = {
                                                               ...mockResellerAccount.branding,
                                                               ...body,
                                                             };
                                                             return HttpResponse.json(mockResellerAccount.branding);
                                                           }),

                                                           // GET /api/v1/reseller/users (TODO(api:T14.03))
                                                           http.get(/\/api\/v1\/reseller\/users$/, () => {
                                                             return HttpResponse.json(mockResellerUsers);
                                                           }),

                                                           // POST /api/v1/reseller/users (TODO(api:T14.04))
                                                           http.post(/\/api\/v1\/reseller\/users$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newUser = {
                                                               id: `ru-${Date.now()}`,
                                                               reseller_id: mockResellerAccount.id,
                                                               username: body.username,
                                                               email: body.email,
                                                               domain: body.domain,
                                                               plan_name: body.plan_name || "Client Starter",
                                                               disk_used_mb: 0,
                                                               status: "active" as const,
                                                               created_at: new Date().toISOString(),
                                                             };
                                                             mockResellerUsers.push(newUser);
                                                             mockResellerAccount.accounts_count += 1;
                                                             return HttpResponse.json(newUser, { status: 201 });
                                                           }),

                                                           // POST /api/v1/reseller/users/:id/status (TODO(api:T14.07))
                                                           http.post(/\/api\/v1\/reseller\/users\/([^/]+)\/status$/, async ({ params, request }) => {
                                                             const id = String(params[0]);
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const user = mockResellerUsers.find((u) => u.id === id);
                                                             if (user) {
                                                               user.status = body.status;
                                                             }
                                                             return HttpResponse.json({ success: true, status: body.status });
                                                           }),

                                                           // DELETE /api/v1/reseller/users/:id (TODO(api:T14.03))
                                                           http.delete(/\/api\/v1\/reseller\/users\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockResellerUsers = mockResellerUsers.filter((u) => u.id !== id);
                                                             mockResellerAccount.accounts_count = Math.max(0, mockResellerAccount.accounts_count - 1);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/reseller/plans (TODO(api:T14.05))
                                                           http.get(/\/api\/v1\/reseller\/plans$/, () => {
                                                             return HttpResponse.json(mockResellerPlans);
                                                           }),

                                                           // POST /api/v1/reseller/plans (TODO(api:T14.05))
                                                           http.post(/\/api\/v1\/reseller\/plans$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newPlan = {
                                                               id: `rp-${Date.now()}`,
                                                               reseller_id: mockResellerAccount.id,
                                                               name: body.name,
                                                               disk_mb: Number(body.disk_mb) || 5120,
                                                               bandwidth_mb: Number(body.bandwidth_mb) || 51200,
                                                               domains_limit: Number(body.domains_limit) || 1,
                                                               databases_limit: Number(body.databases_limit) || 1,
                                                               mailboxes_limit: Number(body.mailboxes_limit) || 1,
                                                               created_at: new Date().toISOString(),
                                                             };
                                                             mockResellerPlans.push(newPlan);
                                                             return HttpResponse.json(newPlan, { status: 201 });
                                                           }),

                                                           // DELETE /api/v1/reseller/plans/:id (TODO(api:T14.05))
                                                           http.delete(/\/api\/v1\/reseller\/plans\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockResellerPlans = mockResellerPlans.filter((p) => p.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/admin/resellers (TODO(api:T14.01))
                                                           http.get(/\/api\/v1\/admin\/resellers$/, () => {
                                                             return HttpResponse.json(mockAdminResellers);
                                                           }),

                                                           // POST /api/v1/admin/resellers (TODO(api:T14.01))
                                                           http.post(/\/api\/v1\/admin\/resellers$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const pkg = mockResellerPackages.find((p) => p.id === body.package_id) || mockResellerPackages[0];
                                                             const newReseller = {
                                                               id: `reseller-${Date.now()}`,
                                                               username: body.username,
                                                               email: body.email,
                                                               company_name: body.company_name,
                                                               package_id: pkg.id,
                                                               package_name: pkg.name,
                                                               status: "active" as const,
                                                               accounts_count: 0,
                                                               disk_used_gb: 0,
                                                               bandwidth_used_gb: 0,
                                                               branding: {
                                                                 brand_name: body.company_name,
                                                                 logo_url: "https://example.com/logo.png",
                                                                 primary_color: "#2563eb",
                                                                 support_email: body.email,
                                                                 custom_login_message: "Welcome to your control panel",
                                                               },
                                                               created_at: new Date().toISOString(),
                                                             };
                                                             mockAdminResellers.push(newReseller);
                                                             return HttpResponse.json(newReseller, { status: 201 });
                                                           }),

                                                           // POST /api/v1/admin/resellers/:id/status (TODO(api:T14.07))
                                                           http.post(/\/api\/v1\/admin\/resellers\/([^/]+)\/status$/, async ({ params, request }) => {
                                                             const id = String(params[0]);
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const reseller = mockAdminResellers.find((r) => r.id === id);
                                                             if (reseller) {
                                                               reseller.status = body.status;
                                                             }
                                                             return HttpResponse.json({ success: true, status: body.status });
                                                           }),

                                                           // DELETE /api/v1/admin/resellers/:id (TODO(api:T14.01))
                                                           http.delete(/\/api\/v1\/admin\/resellers\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockAdminResellers = mockAdminResellers.filter((r) => r.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),

                                                           // GET /api/v1/admin/reseller-packages (TODO(api:T14.02))
                                                           http.get(/\/api\/v1\/admin\/reseller-packages$/, () => {
                                                             return HttpResponse.json(mockResellerPackages);
                                                           }),

                                                           // POST /api/v1/admin/reseller-packages (TODO(api:T14.02))
                                                           http.post(/\/api\/v1\/admin\/reseller-packages$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newPkg = {
                                                               id: `pkg-${Date.now()}`,
                                                               name: body.name,
                                                               max_accounts: Number(body.max_accounts) || 25,
                                                               max_disk_gb: Number(body.max_disk_gb) || 100,
                                                               max_bandwidth_gb: Number(body.max_bandwidth_gb) || 1000,
                                                               allow_overselling: Boolean(body.allow_overselling),
                                                               allowed_plans: ["all"],
                                                               created_at: new Date().toISOString(),
                                                             };
                                                             mockResellerPackages.push(newPkg);
                                                             return HttpResponse.json(newPkg, { status: 201 });
                                                           }),

                                                           // DELETE /api/v1/admin/reseller-packages/:id (TODO(api:T14.02))
                                                           http.delete(/\/api\/v1\/admin\/reseller-packages\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockResellerPackages = mockResellerPackages.filter((p) => p.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),
...(() => {
                                                         // Migration & Transfer Handlers (T15.04, T15.05, T15.06)
                                                         let mockTransferJobs: Array<{
                                                           id: string;
                                                           source_type: "cpanel" | "cyberpanel" | "openpanel" | "builtpanel_archive";
                                                           source_host?: string;
                                                           tenant_username: string;
                                                           target_domain: string;
                                                           status: "pending" | "running" | "completed" | "failed" | "cancelled";
                                                           progress_percent: number;
                                                           current_step: string;
                                                           size_bytes?: number;
                                                           created_at: string;
                                                           updated_at: string;
                                                           steps: Array<{
                                                             id: string;
                                                             name: string;
                                                             status: "pending" | "running" | "completed" | "failed";
                                                             started_at?: string;
                                                             completed_at?: string;
                                                             details?: string;
                                                           }>;
                                                           logs: string[];
                                                         }> = [
                                                           {
                                                             id: "TRF-90241A",
                                                             source_type: "cpanel" as const,
                                                             source_host: "cpanel.legacy-host.com",
                                                             tenant_username: "apex_store",
                                                             target_domain: "apexstore.com",
                                                             status: "running" as const,
                                                             progress_percent: 65,
                                                             current_step: "Restoring MySQL Databases",
                                                             size_bytes: 858993459,
                                                             created_at: new Date(Date.now() - 360000).toISOString(),
                                                             updated_at: new Date().toISOString(),
                                                             steps: [
                                                               {
                                                                 id: "step-1",
                                                                 name: "Connecting to Remote Server via SSH",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 360000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 350000).toISOString(),
                                                                 details: "Authenticated successfully as root via port 22",
                                                               },
                                                               {
                                                                 id: "step-2",
                                                                 name: "Generating & Fetching cpmove Backup Archive",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 350000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 210000).toISOString(),
                                                                 details: "Transferred 819 MB archive (cpmove-apex_store.tar.gz)",
                                                               },
                                                               {
                                                                 id: "step-3",
                                                                 name: "Extracting Archive & Parsing Metadata",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 210000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 170000).toISOString(),
                                                                 details: "Parsed 3 domains, 2 MySQL DBs, 5 mailboxes, and 12 cron entries",
                                                               },
                                                               {
                                                                 id: "step-4",
                                                                 name: "Restoring MySQL Databases & Users",
                                                                 status: "running" as const,
                                                                 started_at: new Date(Date.now() - 170000).toISOString(),
                                                                 details: "Importing apex_store_prod (184 MB) table structure & data",
                                                               },
                                                               {
                                                                 id: "step-5",
                                                                 name: "Restoring Web Root & Vhost Configuration",
                                                                 status: "pending" as const,
                                                               },
                                                               {
                                                                 id: "step-6",
                                                                 name: "Configuring DNS & SSL Certificates",
                                                                 status: "pending" as const,
                                                               },
                                                               {
                                                                 id: "step-7",
                                                                 name: "Verifying Tenant Services & Health Check",
                                                                 status: "pending" as const,
                                                               },
                                                             ],
                                                             logs: [
                                                               "Connecting to remote host cpanel.legacy-host.com on port 22...",
                                                               "SSH handshake complete. Key accepted.",
                                                               "Checking source cPanel version: 110.0.12 (Linux x86_64)",
                                                               "Triggering /scripts/pkgacct apex_store --skiphomedir=0...",
                                                               "pkgacct completed in 42s. Archive size: 819,200 KB.",
                                                               "Starting streaming rsync transfer to /var/builtpanel/transfers/cpmove-apex_store.tar.gz...",
                                                               "Transfer completed: 858,993,459 bytes received (speed 38.4 MB/s).",
                                                               "Validating SHA256 checksum... OK.",
                                                               "Extracting archive metadata from cp/apex_store yaml manifest...",
                                                               "Discovered primary domain: apexstore.com (document_root: public_html)",
                                                               "Provisioning Built Panel tenant apex_store with linux user and cgroup limits...",
                                                               "RESTORED user apex_store (UID 1042, GID 1042)",
                                                               "Extracting MySQL dumps: apex_store_prod.sql.gz, apex_store_cache.sql.gz...",
                                                               "Creating MySQL user apex_store_u with grant on apex_store_prod.*...",
                                                               "Importing database apex_store_prod (table count: 48)...",
                                                             ],
                                                           },
                                                           {
                                                             id: "TRF-88192C",
                                                             source_type: "cyberpanel" as const,
                                                             source_host: "198.51.100.24",
                                                             tenant_username: "agency_portal",
                                                             target_domain: "agencyportal.io",
                                                             status: "completed" as const,
                                                             progress_percent: 100,
                                                             current_step: "Migration Completed Successfully",
                                                             size_bytes: 429496729,
                                                             created_at: new Date(Date.now() - 7200000).toISOString(),
                                                             updated_at: new Date(Date.now() - 6900000).toISOString(),
                                                             steps: [
                                                               {
                                                                 id: "step-1",
                                                                 name: "Connecting to Remote Server via SSH",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 7200000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 7190000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-2",
                                                                 name: "Generating & Fetching CyberPanel Backup",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 7190000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 7080000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-3",
                                                                 name: "Extracting Archive & Parsing Metadata",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 7080000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 7040000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-4",
                                                                 name: "Restoring MySQL Databases & Users",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 7040000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 6980000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-5",
                                                                 name: "Restoring Web Root & Vhost Configuration",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 6980000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 6930000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-6",
                                                                 name: "Configuring DNS & SSL Certificates",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 6930000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 6910000).toISOString(),
                                                               },
                                                               {
                                                                 id: "step-7",
                                                                 name: "Verifying Tenant Services & Health Check",
                                                                 status: "completed" as const,
                                                                 started_at: new Date(Date.now() - 6910000).toISOString(),
                                                                 completed_at: new Date(Date.now() - 6900000).toISOString(),
                                                                 details: "All HTTP/HTTPS endpoints healthy with status 200 OK",
                                                               },
                                                             ],
                                                             logs: [
                                                               "Connecting to remote CyberPanel instance 198.51.100.24:22...",
                                                               "Transfer initiated for agency_portal...",
                                                               "Fetching backup archive: 429,496,729 bytes transferred.",
                                                               "Archive extracted and verified.",
                                                               "RESTORED database agency_portal_main (size: 42 MB).",
                                                               "RESTORED public_html web files (file count: 3,420).",
                                                               "Generated Caddy virtual host configuration for agencyportal.io.",
                                                               "ACME TLS certificate provisioned.",
                                                               "SUCCESS: Migration completed in 5m 0s. All services live.",
                                                             ],
                                                           },
                                                         ];

                                                         return [
                                                           // GET /api/v1/migration/transfers (TODO(api:T15.06))
                                                           http.get(/\/api\/v1\/admin\/migration\/transfers$/, () => {
                                                             return HttpResponse.json(mockTransferJobs);
                                                           }),

                                                           // GET /api/v1/migration/transfers/:id (TODO(api:T15.06))
                                                           http.get(/\/api\/v1\/admin\/migration\/transfers\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             const job = mockTransferJobs.find((j) => j.id === id);
                                                             if (!job) return new HttpResponse(null, { status: 404 });
                                                             return HttpResponse.json(job);
                                                           }),

                                                           // POST /api/v1/migration/transfers (TODO(api:T15.04), TODO(api:T15.05))
                                                           http.post(/\/api\/v1\/admin\/migration\/transfers$/, async ({ request }) => {
                                                             const body = (await request.json().catch(() => ({}))) as any;
                                                             const newJob = {
                                                               id: `TRF-${Math.floor(10000 + Math.random() * 90000)}X`,
                                                               source_type: body.source_type || "cpanel",
                                                               source_host: body.remote_host || "127.0.0.1",
                                                               tenant_username: body.tenant_username,
                                                               target_domain: body.target_domain,
                                                               status: "running" as const,
                                                               progress_percent: 15,
                                                               current_step: "Connecting to Remote Server via SSH",
                                                               size_bytes: 104857600,
                                                               created_at: new Date().toISOString(),
                                                               updated_at: new Date().toISOString(),
                                                               steps: [
                                                                 {
                                                                   id: "step-1",
                                                                   name: "Connecting to Remote Server via SSH",
                                                                   status: "running" as const,
                                                                   started_at: new Date().toISOString(),
                                                                   details: `Connecting to ${body.remote_host || "source"}...`,
                                                                 },
                                                                 {
                                                                   id: "step-2",
                                                                   name: "Generating & Fetching Archive",
                                                                   status: "pending" as const,
                                                                 },
                                                                 {
                                                                   id: "step-3",
                                                                   name: "Extracting Archive & Parsing Metadata",
                                                                   status: "pending" as const,
                                                                 },
                                                                 {
                                                                   id: "step-4",
                                                                   name: "Restoring MySQL Databases & Users",
                                                                   status: "pending" as const,
                                                                 },
                                                                 {
                                                                   id: "step-5",
                                                                   name: "Restoring Web Root & Vhost Configuration",
                                                                   status: "pending" as const,
                                                                 },
                                                                 {
                                                                   id: "step-6",
                                                                   name: "Configuring DNS & SSL Certificates",
                                                                   status: "pending" as const,
                                                                 },
                                                                 {
                                                                   id: "step-7",
                                                                   name: "Verifying Tenant Services",
                                                                   status: "pending" as const,
                                                                 },
                                                               ],
                                                               logs: [
                                                                 `Initiating migration transfer from ${body.source_type} at ${body.remote_host || "local"}...`,
                                                                 `Connecting via SSH to ${body.remote_user || "root"}@${body.remote_host}:${body.remote_ssh_port || 22}...`,
                                                                 "SSH connection established. Handshake verified.",
                                                                 `Target tenant account: ${body.tenant_username} (@${body.target_domain}).`,
                                                               ],
                                                             };
                                                             mockTransferJobs.unshift(newJob);
                                                             return HttpResponse.json(newJob, { status: 201 });
                                                           }),

                                                           // POST /api/v1/migration/transfers/:id/retry (TODO(api:T15.06))
                                                           http.post(/\/api\/v1\/admin\/migration\/transfers\/([^/]+)\/retry$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             const job = mockTransferJobs.find((j) => j.id === id);
                                                             if (job) {
                                                               job.status = "running";
                                                               job.progress_percent = 20;
                                                               job.current_step = "Connecting to Remote Server via SSH";
                                                               job.logs.push("Retrying migration transfer from step 1...");
                                                             }
                                                             return HttpResponse.json({ success: true });
                                                           }),

                                                           // POST /api/v1/migration/transfers/:id/cancel (TODO(api:T15.06))
                                                           http.post(/\/api\/v1\/admin\/migration\/transfers\/([^/]+)\/cancel$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             const job = mockTransferJobs.find((j) => j.id === id);
                                                             if (job) {
                                                               job.status = "cancelled";
                                                               job.current_step = "Transfer Aborted by User";
                                                               job.logs.push("WARNING: Transfer cancelled by operator.");
                                                             }
                                                             return HttpResponse.json({ success: true });
                                                           }),

                                                           // DELETE /api/v1/migration/transfers/:id (TODO(api:T15.06))
                                                           http.delete(/\/api\/v1\/admin\/migration\/transfers\/([^/]+)$/, ({ params }) => {
                                                             const id = String(params[0]);
                                                             mockTransferJobs = mockTransferJobs.filter((j) => j.id !== id);
                                                             return new HttpResponse(null, { status: 204 });
                                                           }),
                                                         ];
                                                       })(),
                                                         ];
                                                       })(),
                                                         ];
                                                       })(),
                                                     ];
                                                   })(),
                                              // ===================================                                              // T18.06 — Branding & Live Theming Handlers (TODO(api:T18.06))
                                              // ===================================
  ...(() => {
                                                let mockBrandingState = {
                                                  companyName: "Built Panel",
                                                  logoUrl: "",
                                                  faviconUrl: "",
                                                  accentColor: "#3b82f6",
                                                  loginTitle: "Built Panel",
                                                  loginSubtitle: "Sign in to control your web hosting infrastructure",
                                                  loginFooterText: "Built Panel Cloud Infrastructure & Management",
                                                  nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
                                                  customCss: "",
                                                };

                                                return [
                                                  // GET /api/v1/branding (TODO(api:T18.06))
                                                  http.get(/\/api\/v1\/branding$/, () => {
                                                    return HttpResponse.json(mockBrandingState);
                                                  }),

                                                  // PUT /api/v1/branding (TODO(api:T18.06))
                                                  http.put(/\/api\/v1\/branding$/, async ({ request }) => {
                                                    const body = (await request.json().catch(() => ({}))) as any;
                                                    mockBrandingState = {
                                                      ...mockBrandingState,
                                                      ...body,
                                                    };
                                                    return HttpResponse.json(mockBrandingState);
                                                  }),
                                                ];
                                              })(),
                                              // ===================================                                              // T18.07: Onboarding & Tour Handlers
                                              // ===================================
  ...(() => {
                                                let adminDismissed = false;
                                                let clientDismissed = false;
                                                let completedSteps = ["add_node"];

                                                return [
                                                  // TODO(api:T18.07): Get admin quickstart onboarding status
                                                  http.get("/api/v1/onboarding/admin", () => {
                                                    return HttpResponse.json({
                                                      dismissed: adminDismissed,
                                                      completed_steps: completedSteps,
                                                    });
                                                  }),

                                                  // TODO(api:T18.07): Dismiss admin quickstart checklist
                                                  http.post("/api/v1/onboarding/admin/dismiss", () => {
                                                    adminDismissed = true;
                                                    return HttpResponse.json({ ok: true, dismissed: true });
                                                  }),

                                                  // TODO(api:T18.07): Toggle admin quickstart checklist step
                                                  http.post("/api/v1/onboarding/admin/step/:id/toggle", ({ params }) => {
                                                    const id = String(params.id);
                                                    if (completedSteps.includes(id)) {
                                                      completedSteps = completedSteps.filter((s) => s !== id);
                                                    } else {
                                                      completedSteps.push(id);
                                                    }
                                                    return HttpResponse.json({ ok: true, completed_steps: completedSteps });
                                                  }),

                                                  // TODO(api:T18.07): Get client welcome tour status
                                                  http.get("/api/v1/onboarding/client", () => {
                                                    return HttpResponse.json({
                                                      dismissed: clientDismissed,
                                                    });
                                                  }),

                                                  // TODO(api:T18.07): Dismiss client welcome tour
                                                  http.post("/api/v1/onboarding/client/dismiss", () => {
                                                    clientDismissed = true;
                                                    return HttpResponse.json({ ok: true, dismissed: true });
                                                  }),
                                                ];
                                              })(),
                                              // ===================================                                              // T18.10: Panel Update Handlers
                                              // ===================================
  ...(() => {
    // The panel's own update: what runs, what is on offer, "Update now" (a run that finishes by itself).
    let lastCheckedAt: string | null = null;
    let run: Record<string, string> | null = null;
    const history: Record<string, string>[] = [];
    const offer = {
      version: "0.2.0",
      release_date: "2026-11-20T10:00:00Z",
      channel: "stable",
      ed25519_signature: "mock-signature",
      key_id: "release-1",
      checksum_sha256: "",
      changelog: ["Per-domain address blocking", "A one-click update"],
      required_migrations: [],
      can_rollback: true,
    };
    let installed = "0.1.0";
    const autoOn = true;
    return [
      http.get("/api/v1/admin/update/status", () =>
        HttpResponse.json({
          current_version: installed,
          channel: "stable",
          last_checked_at: lastCheckedAt,
          available_update: lastCheckedAt && installed !== offer.version ? offer : null,
          previous_version: null,
          can_rollback: false,
          can_apply: true,
          run,
          auto: { enabled: autoOn, delay_days: 3, hour_utc: 3, scheduled_for: autoOn && lastCheckedAt && installed !== offer.version ? "2026-11-26T03:00:00Z" : null },
          history,
        })
      ),
      http.post("/api/v1/admin/update/check", () => {
        lastCheckedAt = new Date().toISOString();
        return HttpResponse.json({ checked: true, available: installed !== offer.version, update: installed !== offer.version ? offer : null });
      }),
      http.post("/api/v1/admin/update/apply", async ({ request }) => {
        const body = (await request.json()) as { version?: string };
        if (body.version !== offer.version) {
          return HttpResponse.json({ code: "no_such_update", detail: "That is not the version on offer." }, { status: 409 });
        }
        const from = installed;
        run = { state: "running", step: "downloading", version: offer.version, from_version: from, message: "", updated_at: new Date().toISOString() };
        setTimeout(() => {
          installed = offer.version;
          run = { state: "completed", step: "completed", version: offer.version, from_version: from, message: `Updated from ${from} to ${offer.version}.`, updated_at: new Date().toISOString() };
          history.unshift({ id: `upd-${Date.now()}`, from_version: from, to_version: offer.version, channel: "stable", status: "success", timestamp: new Date().toISOString(), details: "The release was verified against its signed manifest and installed." });
        }, 3000);
        return HttpResponse.json({ ok: true, status: "started", version: offer.version }, { status: 202 });
      }),
    ];
  })(),
  ...(() => {
    // The administrator's app list ("Install an App"): offering a software to customers and taking it back.
    const apps = [
      { slug: "redis", name: "Redis", category: "databases", category_name: "Databases", description: "In-memory data store used as cache and message broker.", license: "BSD-3-Clause", homepage: "https://redis.io", kind: "container", version: "8.0.2", provided_by: "", reason: "", memory_mb: 256, offered: false },
      { slug: "postgresql", name: "PostgreSQL", category: "databases", category_name: "Databases", description: "The relational database.", license: "PostgreSQL", homepage: "https://postgresql.org", kind: "container", version: "18.1", provided_by: "", reason: "", memory_mb: 512, offered: true },
      { slug: "nginx", name: "Nginx", category: "web_servers", category_name: "Web servers", description: "Web server and reverse proxy.", license: "BSD-2-Clause", homepage: "https://nginx.org", kind: "container", version: "1.29.0", provided_by: "", reason: "", memory_mb: 128, offered: false },
      { slug: "php", name: "PHP", category: "server_side_scripting", category_name: "Server side scripting", description: "PHP versions are part of the panel.", license: "PHP-3.01", homepage: "https://php.net", kind: "builtin", version: "", provided_by: "Domains, PHP settings", reason: "", memory_mb: 0, offered: false },
    ];
    return [
      http.get("/api/v1/admin/marketplace", () => HttpResponse.json(apps)),
      http.post("/api/v1/admin/marketplace/:slug", ({ params }) => {
        const a = apps.find((x) => x.slug === params.slug);
        if (!a) return new HttpResponse(null, { status: 404 });
        a.offered = true;
        return new HttpResponse(null, { status: 204 });
      }),
      http.delete("/api/v1/admin/marketplace/:slug", ({ params }) => {
        const a = apps.find((x) => x.slug === params.slug);
        if (!a) return new HttpResponse(null, { status: 404 });
        a.offered = false;
        return new HttpResponse(null, { status: 204 });
      }),
    ];
  })(),
                                              // ===================================                                              // T18.11: API Tokens & Scoped Execution
                                              // ===================================
  ...(() => {
                                                let mockTokens: Array<{
                                                  id: string;
                                                  name: string;
                                                  token_preview: string;
                                                  scopes: string[];
                                                  created_at: string;
                                                  expires_at: string | null;
                                                  last_used_at: string | null;
                                                }> = [
                                                  {
                                                    id: "tok-whmcs-01",
                                                    name: "WHMCS Primary Production Node",
                                                    token_preview: "dhp_live_9f82...104a",
                                                    scopes: ["users:read", "users:write", "billing:whmcs"],
                                                    created_at: new Date(Date.now() - 86400000 * 14).toISOString(),
                                                    expires_at: null,
                                                    last_used_at: new Date(Date.now() - 3600000).toISOString(),
                                                  },
                                                  {
                                                    id: "tok-metrics-02",
                                                    name: "Prometheus Exporter Scraper",
                                                    token_preview: "dhp_live_4a11...99bc",
                                                    scopes: ["users:read", "domains:read"],
                                                    created_at: new Date(Date.now() - 86400000 * 30).toISOString(),
                                                    expires_at: new Date(Date.now() + 86400000 * 60).toISOString(),
                                                    last_used_at: new Date(Date.now() - 60000).toISOString(),
                                                  },
                                                ];

                                                return [
                                                  // TODO(api:T18.11): List API tokens
                                                  http.get("/api/v1/tokens", () => {
                                                    return HttpResponse.json({ tokens: mockTokens });
                                                  }),

                                                  // TODO(api:T18.11): Create API token
                                                  http.post("/api/v1/tokens", async ({ request }) => {
                                                    const body = (await request.json()) as {
                                                      name: string;
                                                      scopes: string[];
                                                      expiry_days: number | null;
                                                    };

                                                    const hex = Math.random().toString(36).substring(2, 10);
                                                    const secret = `dhp_live_${hex}${Date.now().toString(36)}`;
                                                    const preview = `dhp_live_${hex.substring(0, 4)}...${secret.slice(-4)}`;

                                                    const expiresAt = body.expiry_days
                                                      ? new Date(Date.now() + body.expiry_days * 86400000).toISOString()
                                                      : null;

                                                    const newToken = {
                                                      id: `tok-${Date.now()}`,
                                                      name: body.name || "API Access Token",
                                                      token_preview: preview,
                                                      scopes: body.scopes || ["users:read"],
                                                      created_at: new Date().toISOString(),
                                                      expires_at: expiresAt,
                                                      last_used_at: null,
                                                    };

                                                    mockTokens.unshift(newToken);

                                                    return HttpResponse.json({
                                                      token: newToken,
                                                      secret_key: secret,
                                                    });
                                                  }),

                                                  // TODO(api:T18.11): Revoke API token
                                                  http.delete("/api/v1/tokens/:id", ({ params }) => {
                                                    mockTokens = mockTokens.filter((t) => t.id !== params.id);
                                                    return HttpResponse.json({ ok: true, revoked: true });
                                                  }),

                                                  // TODO(api:T18.11): Test call validating scope authorization (DoD)
                                                  http.post("/api/v1/tokens/test-call", async ({ request }) => {
                                                    const body = (await request.json()) as {
                                                      token_id: string;
                                                      endpoint: string;
                                                      method: string;
                                                    };

                                                    const token = mockTokens.find((t) => t.id === body.token_id);
                                                    if (!token) {
                                                      return HttpResponse.json(
                                                        { status: 401, message: "Unauthorized: Invalid or revoked Bearer token" },
                                                        { status: 401 }
                                                      );
                                                    }

                                                    // Endpoint 1: GET /users requires "users:read"
                                                    if (body.endpoint === "/users") {
                                                      if (!token.scopes.includes("users:read")) {
                                                        return HttpResponse.json(
                                                          { status: 403, message: "Forbidden: Token lacks required scope 'users:read'" },
                                                          { status: 403 }
                                                        );
                                                      }
                                                      return HttpResponse.json({
                                                        status: 200,
                                                        message: "Token authorized for scope 'users:read'",
                                                        data: {
                                                          users_count: 24,
                                                          users: [
                                                            { id: "usr-01", username: "admin", role: "super_admin" },
                                                            { id: "usr-02", username: "client_alpha", role: "client" },
                                                            { id: "usr-03", username: "solaris", role: "client" },
                                                          ],
                                                        },
                                                      });
                                                    }

                                                    // Endpoint 2: POST /billing/whmcs/suspend requires "billing:whmcs"
                                                    if (body.endpoint === "/billing/whmcs/suspend") {
                                                      if (!token.scopes.includes("billing:whmcs")) {
                                                        return HttpResponse.json(
                                                          { status: 403, message: "Forbidden: Token lacks required scope 'billing:whmcs'" },
                                                          { status: 403 }
                                                        );
                                                      }
                                                      return HttpResponse.json({
                                                        status: 200,
                                                        message: "Token authorized for scope 'billing:whmcs'",
                                                        data: {
                                                          action: "account_suspended",
                                                          tenant: "client_alpha",
                                                          reason: "Invoice overdue",
                                                          timestamp: new Date().toISOString(),
                                                        },
                                                      });
                                                    }

                                                    // Endpoint 3: POST /admin/nodes requires "cluster:admin"
                                                    if (body.endpoint === "/admin/nodes") {
                                                      if (!token.scopes.includes("cluster:admin")) {
                                                        return HttpResponse.json(
                                                          {
                                                            status: 403,
                                                            message: "Forbidden: Token lacks required scope 'cluster:admin'",
                                                            required_scope: "cluster:admin",
                                                          },
                                                          { status: 200 } // returned as JSON payload in tester
                                                        );
                                                      }
                                                      return HttpResponse.json({
                                                        status: 200,
                                                        message: "Token authorized for node enrollment",
                                                      });
                                                    }

                                                    return HttpResponse.json({
                                                      status: 404,
                                                      message: "Endpoint not found",
                                                    });
                                                  }),
                                                ];
                                              })(),

                                              // ===================================                                              // T18.12 — Support Report Diagnostic Bundle (TODO(api:T18.12))
                                              // ===================================
                                              http.get(/\/api(?:\/v1\/admin)?\/support\/report$/, () => {
                                                return HttpResponse.json({
                                                  metadata: {
                                                    bundle_id: "diag-20261114-8841",
                                                    generated_at: new Date().toISOString(),
                                                    panel_version: "0.1.0",
                                                    node_id: "node-primary-ams01",
                                                    environment: "production",
                                                  },
                                                  system: {
                                                    os: "Ubuntu 24.04 LTS (Noble Numbat)",
                                                    kernel: "6.8.0-40-generic",
                                                    architecture: "x86_64",
                                                    uptime_seconds: 489201,
                                                    load_average: [0.18, 0.24, 0.28],
                                                    memory: { total_mb: 16384, used_mb: 6144, free_mb: 10240 },
                                                    disk: { total_gb: 512, used_gb: 142, available_gb: 370 },
                                                  },
                                                  services: [
                                                    { name: "nginx", status: "active", uptime: "5d 14h", description: "Edge Reverse Proxy" },
                                                    { name: "postgresql", status: "active", uptime: "5d 14h", description: "Relational Storage" },
                                                    { name: "redis", status: "active", uptime: "5d 14h", description: "In-memory cache & pub/sub" },
                                                    { name: "docker", status: "active", uptime: "5d 14h", description: "Container Runtime" },
                                                    { name: "coraza-waf", status: "active", uptime: "5d 14h", description: "Web Application Firewall" },
                                                  ],
                                                  sanitized_config: {
                                                    database: {
                                                      driver: "postgres",
                                                      host: "127.0.0.1",
                                                      port: 5432,
                                                      database: "builtpanel_db",
                                                      password: "[REDACTED_CONFIDENTIAL]",
                                                    },
                                                    redis: {
                                                      host: "127.0.0.1",
                                                      port: 6379,
                                                      password: "[REDACTED_CONFIDENTIAL]",
                                                    },
                                                    cluster: { mode: "standalone_master", enrolled_nodes: 3 },
                                                    storage: {
                                                      data_path: "/var/lib/builtpanel/data",
                                                      backup_path: "/var/lib/builtpanel/backups",
                                                    },
                                                    features: ["domains", "databases", "waf", "cron", "ssl", "backups"],
                                                  },
                                                  sanitized_logs: [
                                                    {
                                                      timestamp: new Date(Date.now() - 300000).toISOString(),
                                                      level: "info",
                                                      service: "panel-core",
                                                      message: "Health check completed successfully for all active cluster nodes.",
                                                    },
                                                    {
                                                      timestamp: new Date(Date.now() - 180000).toISOString(),
                                                      level: "info",
                                                      service: "auth-gateway",
                                                      message: "User session authenticated via Bearer [REDACTED_TOKEN] from remote IP 192.168.1.10",
                                                    },
                                                    {
                                                      timestamp: new Date(Date.now() - 60000).toISOString(),
                                                      level: "info",
                                                      service: "waf-engine",
                                                      message: "OWASP CRS rule 942100 evaluated without exceptions on vhost demo.example.com",
                                                    },
                                                  ],
                                                });
                                              }),
                                            ];
                                          })(),
...(() => {
  // License Management Handlers (T17.08, T17.09, T17.10, T17.13)
  const mockLicenseState = {
    license_id: "LIC-88241-ENTERPRISE",
    license_key: "DHP-ENT-8492-9182-AA19",
    installation_id: "8c4d9102-5e93-42fa-9b32-fa9921bc810b",
    customer_name: "Apex Cloud Services LLC",
    customer_email: "license@apexcloud.io",
    product_name: "Built Panel Enterprise Edition",
    plan_tier: "enterprise" as const,
    status: "active" as "active" | "grace_period" | "expired" | "suspended" | "restricted",
    issued_at: new Date(Date.now() - 30 * 86400000).toISOString(),
    expires_at: new Date(Date.now() + 335 * 86400000).toISOString(),
    last_heartbeat_at: new Date(Date.now() - 3600000).toISOString(),
    next_refresh_at: new Date(Date.now() + 39600000).toISOString(),
    signing_key_id: "key-ed25519-prod-v1",
    signature_valid: true,
    features: {
      reseller_enabled: true,
      clustering_enabled: true,
      advanced_backups: true,
      commercial_waf: true,
      whitelabel_enabled: true,
      api_access: true,
      max_nodes: 10,
      max_tenants: 100,
      max_domains: 500,
    },
    usage: {
      nodes_used: 2,
      tenants_used: 18,
      domains_used: 42,
    },
    offline_grace_hours_remaining: 72,
  };

  return [
  ];
})(),
...(() => {
  // Admin Dashboard Summary Handlers (T18.01)
  const mockDashboardSummary = {
    nodes: {
      total: 3,
      online: 3,
      avg_cpu_percent: 18,
      avg_ram_percent: 42,
    },
    tenants: {
      total: 24,
      active: 22,
      domains_count: 58,
      databases_count: 36,
    },
    resources: {
      cpu_percent: 24,
      memory_used_mb: 6144,
      memory_total_mb: 16384,
      disk_used_gb: 142,
      disk_total_gb: 512,
      network_rx_mb: 4.8,
      network_tx_mb: 8.2,
    },
    operations: [
      { id: "op-vhost-842", type: "VhostTemplateApply", status: "completed", duration_ms: 120, created_at: new Date().toISOString() },
      { id: "op-ssl-910", type: "AcmeCertificateIssue", status: "completed", duration_ms: 3200, created_at: new Date(Date.now() - 300000).toISOString() },
      { id: "op-db-104", type: "DatabaseBackupExport", status: "completed", duration_ms: 450, created_at: new Date(Date.now() - 600000).toISOString() },
    ],
    security: {
      blocked_threats_24h: 142,
      active_waf_rules: 942,
      ssh_bans: 12,
    },
    license: {
      tier: "Built Panel Enterprise Edition",
      status: "active",
      signature_valid: true,
      expires_in_days: 305,
    },
  };

  return [
  ];
})(),
...(() => {
  // Notification Management Handlers (T18.03, T18.05)
  let mockNotificationsList: Array<{
    id: string;
    title: string;
    message: string;
    severity: "info" | "warning" | "error" | "success";
    category: "security" | "backup" | "ssl" | "system" | "billing";
    read: boolean;
    created_at: string;
    action_url?: string;
    action_label?: string;
  }> = [
    {
      id: "notif-ssl-01",
      title: "SSL Certificate Expiring Soon",
      message: "Let's Encrypt certificate for apexstore.com will expire in 7 days.",
      severity: "warning" as const,
      category: "ssl" as const,
      read: false,
      created_at: new Date(Date.now() - 600000).toISOString(),
      action_url: "/ssl",
      action_label: "Renew Certificate",
    },
    {
      id: "notif-backup-02",
      title: "Nightly Cloud Backup Succeeded",
      message: "420 MB archive transferred to AWS S3 (us-east-1).",
      severity: "success" as const,
      category: "backup" as const,
      read: false,
      created_at: new Date(Date.now() - 3600000).toISOString(),
      action_url: "/backups",
      action_label: "View Snapshot",
    },
    {
      id: "notif-node-03",
      title: "Worker Node lon-02 Connection Warning",
      message: "Node lon-02 agent heartbeat delay detected (latency 420ms).",
      severity: "error" as const,
      category: "system" as const,
      read: false,
      created_at: new Date(Date.now() - 7200000).toISOString(),
      action_url: "/nodes",
      action_label: "Inspect Node",
    },
    {
      id: "notif-waf-04",
      title: "Coraza WAF Blocked SQL Injection",
      message: "Rule 942100 triggered by 198.51.100.42 on /api/login.",
      severity: "warning" as const,
      category: "security" as const,
      read: true,
      created_at: new Date(Date.now() - 86400000).toISOString(),
      action_url: "/waf",
      action_label: "WAF Events",
    },
  ];

  // The inbox, shaped like `NotificationView`; the same routes exist for `client` and `admin`.
  const eventOf = (category: string) => (category === "ssl" ? "ssl.expiring" : category === "backup" ? "backup.finished" : category === "security" ? "security.waf_block" : "system.node");
  let mockChannels: { id: string; kind: string; name: string; target: string; events: string[]; enabled: boolean }[] = [
    { id: "ch_01", kind: "email", name: "Me", target: "admin@enterprise.io", events: [], enabled: true },
  ];
  let mockQuiet: { paused: boolean; snoozed_until: number | null } = { paused: false, snoozed_until: null };
  const NOTIF = /\/api\/v1\/(?:client|admin)\/notifications/;
  const route = (suffix: string) => new RegExp(`${NOTIF.source}${suffix}$`);

  return [
    http.get(route(""), () => {
      return HttpResponse.json({
        items: mockNotificationsList.map((n) => ({
          id: n.id,
          event_type: eventOf(n.category),
          severity: n.severity === "error" ? "critical" : n.severity,
          subject: n.title,
          body: n.message,
          read: n.read,
          created_at: Math.floor(new Date(n.created_at).getTime() / 1000),
          data: null,
        })),
        unread: mockNotificationsList.filter((n) => !n.read).length,
      });
    }),

    // POST .../notifications/read  { ids? }: some, or all
    http.post(route("/read"), async ({ request }) => {
      const body = (await request.json().catch(() => ({}))) as { ids?: string[] | null };
      let updated = 0;
      mockNotificationsList = mockNotificationsList.map((n) => {
        if (!n.read && (!body.ids || body.ids.includes(n.id))) updated++;
        return !body.ids || body.ids.includes(n.id) ? { ...n, read: true } : n;
      });
      return HttpResponse.json({ updated });
    }),

    http.get(route("/events"), () =>
      HttpResponse.json({ events: ["ssl.expiring", "backup.finished", "backup.failed", "security.waf_block", "system.node"] }),
    ),

    http.get(route("/channels"), () => HttpResponse.json(mockChannels)),
    http.post(route("/channels"), async ({ request }) => {
      const body = (await request.json()) as { kind: string; name: string; target: string; events?: string[] };
      const channel = { id: `ch_${Date.now()}`, kind: body.kind, name: body.name, target: body.target, events: body.events ?? [], enabled: true };
      mockChannels = [...mockChannels, channel];
      return HttpResponse.json({ channel, secret: body.kind === "webhook" ? "whsec_mock_once" : null }, { status: 201 });
    }),
    http.patch(route("/channels/([^/]+)"), async ({ params, request }) => {
      const body = (await request.json()) as { enabled?: boolean; events?: string[] };
      mockChannels = mockChannels.map((c) => (c.id === params[0] ? { ...c, ...body } : c));
      return new HttpResponse(null, { status: 204 });
    }),
    http.delete(route("/channels/([^/]+)"), ({ params }) => {
      mockChannels = mockChannels.filter((c) => c.id !== params[0]);
      return new HttpResponse(null, { status: 204 });
    }),

    http.get(route("/quiet"), () => HttpResponse.json(mockQuiet)),
    http.put(route("/quiet"), async ({ request }) => {
      const body = (await request.json()) as { paused: boolean; snooze_minutes?: number };
      mockQuiet = {
        paused: body.paused,
        snoozed_until: body.snooze_minutes ? Math.floor(Date.now() / 1000) + body.snooze_minutes * 60 : null,
      };
      return HttpResponse.json(mockQuiet);
    }),
  ];
})(),
...(() => {
  // Global Search & Command Palette Handlers (T18.09)
  const searchableEntities = [
    {
      id: "domain-apexstore",
      title: "apexstore.com",
      subtitle: "Tenant: apex_store (IP: 198.51.100.24)",
      category: "domains" as const,
      href: "/domains",
      badge: "Domain",
    },
    {
      id: "domain-mysite",
      title: "mysite.org",
      subtitle: "Tenant: user_john (IP: 198.51.100.24)",
      category: "domains" as const,
      href: "/domains",
      badge: "Domain",
    },
    {
      id: "user-apex-store",
      title: "apex_store",
      subtitle: "Pro Plan • apex@store.io",
      category: "users" as const,
      href: "/users",
      badge: "Tenant",
    },
    {
      id: "user-john",
      title: "user_john",
      subtitle: "Standard Plan • john@mysite.org",
      category: "users" as const,
      href: "/users",
      badge: "Tenant",
    },
    {
      id: "db-apex-prod",
      title: "apex_store_prod",
      subtitle: "MySQL 8.4 • 184 MB",
      category: "actions" as const,
      href: "/databases",
      badge: "Database",
    },
  ];

  return [
    // GET /api/v1/search?q=... (TODO(api:T18.09))
    http.get(/\/api\/v1\/search$/, ({ request }) => {
      const url = new URL(request.url);
      const q = (url.searchParams.get("q") || "").toLowerCase().trim();
      if (!q) return HttpResponse.json([]);

      const matched = searchableEntities.filter(
        (e) => e.title.toLowerCase().includes(q) || e.subtitle.toLowerCase().includes(q)
      );
      return HttpResponse.json(matched);
    }),
  ];
})(),
                                        ];
                                      })(),
                                    ];
                                  })(),
                                ];
                              })(),
                            ];
                          })(),
                        ];
                      })(),
                    ];
                  })(),
                ];
              })(),
            ];
          })(),
        ];
      })(),
    // License Authority (LA) Handlers (T17.06) TODO(api:T17.06)
    ...(() => {
      const mockLaLicenses: any[] = [
        {
          id: "lic_01",
          license_key: "DHP-ENT-2026-A1B2-C3D4",
          customer_id: "cust_01",
          customer_name: "Acme Global Hosting",
          customer_email: "billing@acme.com",
          plan_tier: "enterprise",
          status: "active",
          max_nodes: 10,
          max_tenants: 500,
          activations_count: 2,
          max_activations: 5,
          created_at: new Date(Date.now() - 30 * 86400 * 1000).toISOString(),
          expires_at: new Date(Date.now() + 335 * 86400 * 1000).toISOString(),
          revoked_at: null,
          revoke_reason: null,
        },
        {
          id: "lic_02",
          license_key: "DHP-BUS-2026-F9E8-D7C6",
          customer_id: "cust_02",
          customer_name: "Nordic Cloud Services",
          customer_email: "admin@nordiccloud.se",
          plan_tier: "business",
          status: "active",
          max_nodes: 5,
          max_tenants: 100,
          activations_count: 1,
          max_activations: 2,
          created_at: new Date(Date.now() - 60 * 86400 * 1000).toISOString(),
          expires_at: new Date(Date.now() + 305 * 86400 * 1000).toISOString(),
          revoked_at: null,
          revoke_reason: null,
        },
        {
          id: "lic_03",
          license_key: "DHP-STR-2026-X1Y2-Z3W4",
          customer_id: "cust_03",
          customer_name: "Apex DevOps Lab",
          customer_email: "support@apexdev.io",
          plan_tier: "starter",
          status: "revoked",
          max_nodes: 2,
          max_tenants: 25,
          activations_count: 0,
          max_activations: 1,
          created_at: new Date(Date.now() - 90 * 86400 * 1000).toISOString(),
          expires_at: new Date(Date.now() + 275 * 86400 * 1000).toISOString(),
          revoked_at: new Date(Date.now() - 5 * 86400 * 1000).toISOString(),
          revoke_reason: "Payment default or chargeback",
        },
      ];

      const mockLaCustomers: any[] = [
        {
          id: "cust_01",
          name: "Acme Global Hosting",
          email: "billing@acme.com",
          company: "Acme Corp",
          created_at: new Date(Date.now() - 100 * 86400 * 1000).toISOString(),
          licenses_count: 1,
        },
        {
          id: "cust_02",
          name: "Nordic Cloud Services",
          email: "admin@nordiccloud.se",
          company: "Nordic Cloud AB",
          created_at: new Date(Date.now() - 80 * 86400 * 1000).toISOString(),
          licenses_count: 1,
        },
        {
          id: "cust_03",
          name: "Apex DevOps Lab",
          email: "support@apexdev.io",
          company: "Apex Technologies LLC",
          created_at: new Date(Date.now() - 95 * 86400 * 1000).toISOString(),
          licenses_count: 1,
        },
      ];

      const mockLaActivations: any[] = [
        {
          id: "act_01",
          license_id: "lic_01",
          license_key: "DHP-ENT-2026-A1B2-C3D4",
          installation_id: "00000000-0000-0000-0000-0000000000aa",
          server_ip: "167.104.101.81",
          hostname: "cp1.builtpanel.net",
          last_heartbeat_at: new Date().toISOString(),
          status: "active",
        },
        {
          id: "act_02",
          license_id: "lic_01",
          license_key: "DHP-ENT-2026-A1B2-C3D4",
          installation_id: "00000000-0000-0000-0000-0000000000bb",
          server_ip: "167.104.101.82",
          hostname: "worker2.builtpanel.net",
          last_heartbeat_at: new Date(Date.now() - 120 * 1000).toISOString(),
          status: "active",
        },
      ];

      return [
        // Overview (GET /api/v1/la/overview)
        http.get(/\/api\/v1\/la\/overview$/, () => {
          return HttpResponse.json({
            total_licenses: mockLaLicenses.length,
            active_licenses: mockLaLicenses.filter((l) => l.status === "active").length,
            revoked_licenses: mockLaLicenses.filter((l) => l.status === "revoked").length,
            total_customers: mockLaCustomers.length,
            active_activations: mockLaActivations.filter((a) => a.status === "active").length,
          });
        }),

        // List Licenses (GET /api/v1/la/licenses)
        http.get(/\/api\/v1\/la\/licenses$/, () => {
          return HttpResponse.json({
            items: mockLaLicenses,
            total: mockLaLicenses.length,
          });
        }),

        // Issue License (POST /api/v1/la/licenses) (DoD T17.06)
        http.post(/\/api\/v1\/la\/licenses$/, async ({ request }) => {
          const body = (await request.json().catch(() => ({}))) as any;
          const randomPart = Math.random().toString(36).substring(2, 6).toUpperCase();
          const randomPart2 = Math.random().toString(36).substring(2, 6).toUpperCase();
          const tierCode = (body.plan_tier || "ENT").substring(0, 3).toUpperCase();
          const licenseKey = `DHP-${tierCode}-2026-${randomPart}-${randomPart2}`;

          const newLicense = {
            id: `lic_${Date.now()}`,
            license_key: licenseKey,
            customer_id: `cust_${Date.now()}`,
            customer_name: body.customer_name || "New Customer",
            customer_email: body.customer_email || "billing@customer.com",
            plan_tier: body.plan_tier || "enterprise",
            status: "active",
            max_nodes: Number(body.max_nodes) || 5,
            max_tenants: Number(body.max_tenants) || 100,
            activations_count: 0,
            max_activations: Number(body.max_activations) || 1,
            created_at: new Date().toISOString(),
            expires_at: body.validity_days
              ? new Date(Date.now() + body.validity_days * 86400 * 1000).toISOString()
              : new Date(Date.now() + 3650 * 86400 * 1000).toISOString(),
            revoked_at: null,
            revoke_reason: null,
          };

          mockLaLicenses.unshift(newLicense);
          return HttpResponse.json({ success: true, license: newLicense }, { status: 201 });
        }),

        // Revoke License (POST /api/v1/la/licenses/:id/revoke) (DoD T17.06)
        http.post(/\/api\/v1\/la\/licenses\/([^/]+)\/revoke$/, async ({ params, request }) => {
          const { 0: id } = params as any;
          const body = (await request.json().catch(() => ({}))) as any;
          const target = mockLaLicenses.find((l) => l.id === id || l.license_key === id);

          if (!target) {
            return HttpResponse.json({ error: "license_not_found", message: "License not found" }, { status: 404 });
          }

          target.status = "revoked";
          target.revoked_at = new Date().toISOString();
          target.revoke_reason = body.reason || "Administrative revocation";

          return HttpResponse.json({
            success: true,
            license: target,
            message: "License revoked successfully",
          });
        }),

        // List Activations (GET /api/v1/la/activations)
        http.get(/\/api\/v1\/la\/activations$/, () => {
          return HttpResponse.json({
            items: mockLaActivations,
            total: mockLaActivations.length,
          });
        }),

        // List Customers (GET /api/v1/la/customers)
        http.get(/\/api\/v1\/la\/customers$/, () => {
          return HttpResponse.json({
            items: mockLaCustomers,
            total: mockLaCustomers.length,
          });
        }),
      ];
    })(),
  ];
})(),
];
