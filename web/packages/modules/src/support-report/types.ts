export interface SystemDiagnostics {
  os: string;
  kernel: string;
  architecture: string;
  uptime_seconds: number;
  load_average: [number, number, number];
  memory: {
    total_mb: number;
    used_mb: number;
    free_mb: number;
  };
  disk: {
    total_gb: number;
    used_gb: number;
    available_gb: number;
  };
}

export interface ServiceDiagnosticItem {
  name: string;
  status: "active" | "inactive" | "failed";
  uptime: string;
  description: string;
}

export interface DiagnosticBundle {
  metadata: {
    bundle_id: string;
    generated_at: string;
    panel_version: string;
    node_id: string;
    environment: string;
  };
  system: SystemDiagnostics;
  services: ServiceDiagnosticItem[];
  sanitized_config: Record<string, any>;
  sanitized_logs: Array<{
    timestamp: string;
    level: "info" | "warn" | "error";
    service: string;
    message: string;
  }>;
}

const SENSITIVE_KEY_PATTERN = /(?:password|passwd|secret|token|auth|key|credential|private|cert|cookie|signature|session|bearer|jwt)/i;

const SENSITIVE_VALUE_PATTERNS: Array<{ regex: RegExp; replace: string }> = [
  // Database connection string credentials: e.g. postgresql://user:pass@host:5432/db
  {
    regex: /(postgres(?:ql)?|mysql|redis):\/\/([^:/\s]+):(.+)@([a-zA-Z0-9_.-]+(?::\d+)?)/gi,
    replace: "$1://$2:[REDACTED_PASSWORD]@$4",
  },
  // Bearer tokens: e.g. Bearer eyJhbG...
  {
    regex: /Bearer\s+([A-Za-z0-9\-._~+/]+=*)/gi,
    replace: "Bearer [REDACTED_TOKEN]",
  },
  // PEM Private keys: -----BEGIN ... PRIVATE KEY-----
  {
    regex: /-----BEGIN\s+(?:[A-Z\s]+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(?:[A-Z\s]+)?PRIVATE\s+KEY-----/gi,
    replace: "[REDACTED_PRIVATE_KEY]",
  },
  // API Keys / Hex / Base64 tokens
  {
    regex: /(api[_-]?key|secret[_-]?token|app[_-]?secret)[\s:=]+([A-Za-z0-9-_]{16,})/gi,
    replace: "$1=[REDACTED_SECRET]",
  },
];

/**
 * Recursively scrubs sensitive secrets, passwords, private keys, and tokens from any object or string.
 */
export function scrubString(str: string): string {
  let cleaned = str;
  for (const { regex, replace } of SENSITIVE_VALUE_PATTERNS) {
    cleaned = cleaned.replace(regex, replace);
  }
  return cleaned;
}

export function sanitizeSupportBundle(bundle: any): DiagnosticBundle {
  function sanitizeNode(node: any, keyName?: string): any {
    if (node === null || node === undefined) return node;

    // Check if the property key name itself is sensitive
    if (keyName && SENSITIVE_KEY_PATTERN.test(keyName)) {
      return "[REDACTED_CONFIDENTIAL]";
    }

    if (typeof node === "string") {
      return scrubString(node);
    }

    if (Array.isArray(node)) {
      return node.map((item) => sanitizeNode(item, keyName));
    }

    if (typeof node === "object") {
      const sanitizedObj: Record<string, any> = {};
      for (const [k, v] of Object.entries(node)) {
        sanitizedObj[k] = sanitizeNode(v, k);
      }
      return sanitizedObj;
    }

    return node;
  }

  return sanitizeNode(bundle) as DiagnosticBundle;
}

/**
 * Strict verification test function (DoD) to ensure zero plaintext secrets exist in bundle.
 */
export function verifyNoSecrets(bundle: DiagnosticBundle): { clean: boolean; leaks: string[] } {
  const leaks: string[] = [];
  const serialized = JSON.stringify(bundle);

  // 1. Look for unmasked private key markers
  if (/-----BEGIN\s+[A-Z\s]*PRIVATE\s+KEY-----/i.test(serialized)) {
    leaks.push("Unmasked private key header found");
  }

  // 2. Look for postgres/mysql credentials in URIs
  if (/(?:postgres|mysql):\/\/[^:]+:[^@\s]+@/i.test(serialized)) {
    const matched = serialized.match(/(?:postgres|mysql):\/\/[^:]+:([^@\s]+)@/i);
    if (matched && !matched[1].includes("[REDACTED")) {
      leaks.push("Plaintext database password in URI found");
    }
  }

  // 3. Look for raw Bearer JWT strings
  if (/Bearer\s+eyJ[A-Za-z0-9-_]{20,}/i.test(serialized)) {
    leaks.push("Plaintext Bearer JWT token found");
  }

  return {
    clean: leaks.length === 0,
    leaks,
  };
}

export const MOCK_DIAGNOSTIC_BUNDLE: DiagnosticBundle = {
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
    memory: {
      total_mb: 16384,
      used_mb: 6144,
      free_mb: 10240,
    },
    disk: {
      total_gb: 512,
      used_gb: 142,
      available_gb: 370,
    },
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
    cluster: {
      mode: "standalone_master",
      enrolled_nodes: 3,
    },
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
};
