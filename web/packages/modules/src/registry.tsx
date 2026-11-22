import React from "react";
import { LayoutDashboard, ShieldCheck, ShieldAlert, Sparkles, Puzzle, Globe, Users, Settings, FileText, Boxes, Server, Activity, Layers, Lock, FileCode2, Cpu, Database, Folder, Archive, Clock, Terminal, Wrench, Flame, Mail, Briefcase, ArrowRightLeft, KeyRound, Palette, ArrowUpCircle, Key, Network } from "lucide-react";

export interface ModuleManifest {
  name: string;
  title: { en: string; bn: string };
  category: string;
  core: boolean;
  permissions?: string[];
  features?: string[];
}

export interface ModuleNavItem {
  id: string;
  labelKey: string;
  icon: React.ReactNode;
  href: string;
  order?: number;
  permission?: string;
  feature?: string;
}

export interface FrontendModule {
  manifest: ModuleManifest;
  adminNav?: ModuleNavItem[];
  clientNav?: ModuleNavItem[];
}

export const registeredFrontendModules: FrontendModule[] = [
  {
    manifest: {
      name: "admin-tools",
      title: { en: "System tools", bn: "সিস্টেম টুলস" },
      category: "core",
      core: true,
      permissions: ["admin.settings.manage"],
    },
    adminNav: [
      {
        id: "admin-tools",
        labelKey: "adminTools.nav",
        icon: <Wrench className="h-4 w-4" />,
        href: "/system-tools",
        order: 95,
        permission: "admin.settings.manage",
      },
    ],
  },
  {
    manifest: {
      name: "dashboard",
      title: { en: "Dashboard", bn: "ড্যাশবোর্ড" },
      category: "core",
      core: true,
      permissions: ["dashboard.read"],
      features: ["dashboard"],
    },
    adminNav: [
      {
        id: "dashboard",
        labelKey: "common.dashboard",
        icon: <LayoutDashboard className="h-4 w-4" />,
        href: "/",
        order: 10,
      },
    ],
    clientNav: [
      {
        id: "dashboard",
        labelKey: "common.dashboard",
        icon: <LayoutDashboard className="h-4 w-4" />,
        href: "/",
        order: 10,
      },
    ],
  },
  {
    manifest: {
      name: "security",
      title: { en: "Security", bn: "নিরাপত্তা" },
      category: "core",
      core: true,
      permissions: ["security.read"],
      features: ["security"],
    },
    adminNav: [
      {
        id: "security",
        labelKey: "security.nav",
        icon: <ShieldCheck className="h-4 w-4" />,
        href: "/security",
        order: 20,
      },
      {
        id: "security-center",
        labelKey: "securityCenter.nav",
        icon: <ShieldAlert className="h-4 w-4" />,
        href: "/security-center",
        order: 21,
      },
    ],
    clientNav: [
      {
        id: "security",
        labelKey: "security.nav",
        icon: <ShieldCheck className="h-4 w-4" />,
        href: "/security",
        order: 20,
      },
    ],
  },
  {
    manifest: {
      name: "nodes",
      title: { en: "Cluster Nodes", bn: "ক্লাস্টার নোডস" },
      category: "core",
      core: true,
      permissions: ["admin.nodes.read"],
    },
    adminNav: [
      {
        id: "nodes",
        labelKey: "nodes.nav",
        icon: <Server className="h-4 w-4" />,
        href: "/nodes",
        order: 22,
        permission: "admin.nodes.read",
      },
    ],
  },
  {
    manifest: {
      name: "operations",
      title: { en: "Operation Ledger", bn: "অপারেশন লেজার" },
      category: "core",
      core: true,
      permissions: ["admin.operations.read"],
    },
    adminNav: [
      {
        id: "operations",
        labelKey: "operations.nav",
        icon: <Activity className="h-4 w-4" />,
        href: "/operations",
        order: 24,
        permission: "admin.operations.read",
      },
    ],
  },
  {
    manifest: {
      name: "feature-sets",
      title: { en: "Feature Sets", bn: "ফিচার সেট" },
      category: "core",
      core: true,
      permissions: ["admin.plans.read", "admin.plans.manage"],
    },
    adminNav: [
      {
        id: "feature-sets",
        labelKey: "featureSets.nav",
        icon: <Layers className="h-4 w-4" />,
        href: "/feature-sets",
        order: 30,
        permission: "admin.plans.read",
      },
    ],
  },
  {
    manifest: {
      name: "users",
      title: { en: "Users", bn: "ব্যবহারকারী" },
      category: "core",
      core: true,
      permissions: ["admin.users.read", "admin.users.manage"],
    },
    adminNav: [
      {
        id: "users",
        labelKey: "users.nav",
        icon: <Users className="h-4 w-4" />,
        href: "/users",
        order: 21,
        permission: "admin.users.read",
      },
    ],
  },
  {
    manifest: {
      name: "plans",
      title: { en: "Hosting Plans", bn: "হোস্টিং প্ল্যান" },
      category: "core",
      core: true,
      permissions: ["admin.plans.read", "admin.plans.manage"],
    },
    adminNav: [
      {
        id: "plans",
        labelKey: "plans.nav",
        icon: <Boxes className="h-4 w-4" />,
        href: "/plans",
        order: 32,
        permission: "admin.plans.read",
      },
    ],
  },
  {
    manifest: {
      name: "audit",
      title: { en: "Audit Logs", bn: "অডিট লগ" },
      category: "core",
      core: true,
      permissions: ["admin.audit.read"],
    },
    adminNav: [
      {
        id: "audit",
        labelKey: "audit.nav",
        icon: <FileText className="h-4 w-4" />,
        href: "/audit",
        order: 25,
        permission: "admin.audit.read",
      },
    ],
  },
  {
    manifest: {
      name: "demo",
      title: { en: "UI Foundation", bn: "ইউআই ফাউন্ডেশন" },
      category: "core",
      core: true,
      features: ["ui_demo"],
    },
    adminNav: [
      {
        id: "demo",
        labelKey: "UI Foundation (T00.12)",
        icon: <Sparkles className="h-4 w-4 text-primary" />,
        href: "/demo",
        order: 90,
      },
    ],
  },
  {
    manifest: {
      name: "dummy",
      title: { en: "Dummy Module", bn: "ডামি মডিউল" },
      category: "plugins",
      core: false,
      permissions: ["dummy.read"],
      features: ["dummy_feature"],
    },
    adminNav: [
      {
        id: "dummy",
        labelKey: "dummy.nav",
        icon: <Puzzle className="h-4 w-4 text-amber-500" />,
        href: "/dummy",
        order: 50,
        permission: "dummy.read",
        feature: "dummy_feature",
      },
    ],
    clientNav: [
      {
        id: "dummy",
        labelKey: "dummy.nav",
        icon: <Puzzle className="h-4 w-4 text-amber-500" />,
        href: "/dummy",
        order: 50,
        permission: "dummy.read",
        feature: "dummy_feature",
      },
    ],
  },
  {
    manifest: {
      name: "domains",
      title: { en: "Domains", bn: "ডোমেইন" },
      category: "web",
      core: false,
      permissions: ["domains.read", "domains.create", "domains.update", "domains.delete"],
      features: ["domains", "redirects", "docroot"],
    },
    adminNav: [
      {
        id: "domains",
        labelKey: "domains.nav",
        icon: <Globe className="h-4 w-4" />,
        href: "/domains",
        order: 30,
        permission: "domains.read",
        feature: "domains",
      },
    ],
    clientNav: [
      {
        id: "domains",
        labelKey: "domains.nav",
        icon: <Globe className="h-4 w-4" />,
        href: "/domains",
        order: 20,
        permission: "domains.read",
        feature: "domains",
      },
    ],
  },
  {
    manifest: {
      name: "dns",
      title: { en: "DNS Editor", bn: "ডিএনএস এডিটর" },
      category: "web",
      core: false,
      permissions: ["dns.read", "dns.create", "dns.update", "dns.delete"],
      features: ["dns", "bind_export", "bind_import"],
    },
    adminNav: [
      {
        id: "dns",
        labelKey: "dns.nav",
        icon: <Globe className="h-4 w-4" />,
        href: "/dns",
        order: 32,
        permission: "dns.read",
        feature: "dns",
      },
    ],
    clientNav: [
      {
        id: "dns",
        labelKey: "dns.nav",
        icon: <Globe className="h-4 w-4" />,
        href: "/dns",
        order: 22,
        permission: "dns.read",
        feature: "dns",
      },
    ],
  },
  {
    manifest: {
      name: "ssl",
      title: { en: "SSL / TLS", bn: "এসএসএল / টিএলএস" },
      category: "security",
      core: false,
      permissions: ["ssl.read", "ssl.manage", "ssl.upload"],
      features: ["ssl", "custom_ssl", "hsts", "force_https"],
    },
    adminNav: [
      {
        id: "ssl",
        labelKey: "ssl.nav",
        icon: <Lock className="h-4 w-4" />,
        href: "/ssl",
        order: 34,
        permission: "ssl.read",
        feature: "ssl",
      },
    ],
    clientNav: [
      {
        id: "ssl",
        labelKey: "ssl.nav",
        icon: <Lock className="h-4 w-4" />,
        href: "/ssl",
        order: 24,
        permission: "ssl.read",
        feature: "ssl",
      },
    ],
  },
  {
    manifest: {
      name: "vhost",
      title: { en: "VHost Templates", bn: "ভিহোস্ট টেমপ্লেট" },
      category: "webserver",
      core: true,
      permissions: ["admin.vhost.manage"],
    },
    adminNav: [
      {
        id: "vhost",
        labelKey: "vhost.nav",
        icon: <FileCode2 className="h-4 w-4" />,
        href: "/vhost-templates",
        order: 36,
        permission: "admin.vhost.manage",
      },
    ],
  },
  {
    manifest: {
      name: "error-pages",
      title: { en: "Error & Suspended Pages", bn: "এরর ও সাসপেন্ডেড পেজ" },
      category: "webserver",
      core: true,
      permissions: ["admin.templates.manage"],
    },
    adminNav: [
      {
        id: "error-pages",
        labelKey: "errorPages.nav",
        icon: <FileText className="h-4 w-4" />,
        href: "/error-pages",
        order: 37,
        permission: "admin.templates.manage",
      },
    ],
  },
  {
    manifest: {
      name: "php",
      title: { en: "PHP Runtime", bn: "পিএইচপি রানটাইম" },
      category: "webserver",
      core: true,
      permissions: ["php.read", "php.manage"],
      features: ["php_version", "php_ini"],
    },
    adminNav: [
      {
        id: "php",
        labelKey: "php.nav",
        icon: <Cpu className="h-4 w-4" />,
        href: "/php",
        order: 38,
        permission: "php.read",
      },
    ],
    clientNav: [
      {
        id: "php",
        labelKey: "php.nav",
        icon: <Cpu className="h-4 w-4" />,
        href: "/php",
        order: 26,
        permission: "php.read",
        feature: "php_version",
      },
    ],
  },
  {
    manifest: {
      name: "services",
      title: { en: "Web Services", bn: "ওয়েব সার্ভিস" },
      category: "webserver",
      core: true,
      permissions: ["services.read", "services.restart"],
      features: ["services"],
    },
    adminNav: [
      {
        id: "services",
        labelKey: "services.nav",
        icon: <Activity className="h-4 w-4" />,
        href: "/services",
        order: 39,
        permission: "services.read",
      },
    ],
    clientNav: [
      {
        id: "services",
        labelKey: "services.nav",
        icon: <Activity className="h-4 w-4" />,
        href: "/services",
        order: 28,
        permission: "services.read",
        feature: "services",
      },
    ],
  },
  {
    manifest: {
      name: "databases",
      title: { en: "Databases", bn: "ডেটাবেসসমূহ" },
      category: "database",
      core: true,
      permissions: ["databases.read", "databases.manage"],
      features: ["databases"],
    },
    clientNav: [
      {
        id: "databases",
        labelKey: "databases.nav",
        icon: <Database className="h-4 w-4" />,
        href: "/databases",
        order: 25,
        permission: "databases.read",
        feature: "databases",
      },
    ],
    adminNav: [
      {
        id: "databases",
        labelKey: "databases.nav",
        icon: <Database className="h-4 w-4" />,
        href: "/admin/databases",
        order: 29,
        permission: "databases.read",
      },
    ],
  },
  {
    manifest: {
      name: "files",
      title: { en: "File Manager", bn: "ফাইল ম্যানেজার" },
      category: "files",
      core: true,
      permissions: ["files.read", "files.manage"],
      features: ["files"],
    },
    clientNav: [
      {
        id: "files",
        labelKey: "files.nav",
        icon: <Folder className="h-4 w-4" />,
        href: "/files",
        order: 26,
        permission: "files.read",
        feature: "files",
      },
    ],
  },
  {
    manifest: {
      name: "ftp",
      title: { en: "FTP Accounts", bn: "এফটিপি অ্যাকাউন্ট" },
      category: "files",
      core: true,
      permissions: ["ftp.read", "ftp.manage"],
      features: ["ftp"],
    },
    clientNav: [
      {
        id: "ftp",
        labelKey: "ftp.nav",
        icon: <Server className="h-4 w-4" />,
        href: "/ftp",
        order: 27,
        permission: "ftp.read",
        feature: "ftp",
      },
    ],
  },
  {
    manifest: {
      name: "backups",
      title: { en: "Backups", bn: "ব্যাকআপসমূহ" },
      category: "backup",
      core: true,
      permissions: ["backups.read", "backups.manage"],
      features: ["backups"],
    },
    clientNav: [
      {
        id: "backups",
        labelKey: "backups.nav",
        icon: <Archive className="h-4 w-4" />,
        href: "/backups",
        order: 28,
        permission: "backups.read",
        feature: "backups",
      },
    ],
    adminNav: [
      {
        id: "admin-backups",
        labelKey: "adminBackups.nav",
        icon: <Archive className="h-4 w-4" />,
        href: "/admin/backups",
        order: 55,
        permission: "backups.manage",
      },
    ],
  },
  {
    manifest: {
      name: "cron",
      title: { en: "Cron Jobs", bn: "ক্রন জব" },
      category: "tools",
      core: true,
      permissions: ["cron.read", "cron.manage"],
      features: ["cron"],
    },
    clientNav: [
      {
        id: "cron",
        labelKey: "cron.nav",
        icon: <Clock className="h-4 w-4" />,
        href: "/cron",
        order: 30,
        permission: "cron.read",
        feature: "cron",
      },
    ],
  },
  {
    manifest: {
      name: "logs",
      title: { en: "Log Viewer", bn: "লগ ভিউয়ার" },
      category: "monitoring",
      core: true,
      permissions: ["logs.read"],
      features: ["logs"],
    },
    clientNav: [
      {
        id: "logs",
        labelKey: "logs.nav",
        icon: <FileText className="h-4 w-4" />,
        href: "/logs",
        order: 32,
        permission: "logs.read",
        feature: "logs",
      },
    ],
    adminNav: [
      {
        id: "admin-system-logs",
        labelKey: "adminSystemLogs.nav",
        icon: <FileText className="h-4 w-4" />,
        href: "/admin/system-logs",
        order: 58,
        permission: "system.logs.read",
      },
    ],
  },
  {
    manifest: {
      name: "usage",
      title: { en: "Resource Usage", bn: "রিসোর্স ব্যবহার" },
      category: "monitoring",
      core: true,
      permissions: ["usage.read", "admin.usage.read"],
      features: ["usage"],
    },
    clientNav: [
      {
        id: "usage",
        labelKey: "usage.nav",
        icon: <Activity className="h-4 w-4" />,
        href: "/usage",
        order: 34,
        permission: "usage.read",
        feature: "usage",
      },
    ],
    adminNav: [
      {
        id: "admin-usage",
        labelKey: "adminUsage.nav",
        icon: <Activity className="h-4 w-4" />,
        href: "/admin/usage",
        order: 60,
        permission: "admin.usage.read",
      },
    ],
  },
  {
    manifest: {
      name: "processes",
      title: { en: "Process Manager", bn: "প্রসেস ম্যানেজার" },
      category: "tools",
      core: true,
      permissions: ["processes.read", "processes.manage", "admin.processes.manage"],
      features: ["processes"],
    },
    clientNav: [
      {
        id: "processes",
        labelKey: "processes.nav",
        icon: <Terminal className="h-4 w-4" />,
        href: "/processes",
        order: 36,
        permission: "processes.read",
        feature: "processes",
      },
    ],
    adminNav: [
      {
        id: "admin-processes",
        labelKey: "adminProcesses.nav",
        icon: <Terminal className="h-4 w-4" />,
        href: "/admin/processes",
        order: 62,
        permission: "admin.processes.manage",
      },
    ],
  },
  {
    manifest: {
      name: "server-tools",
      title: { en: "Server Tools", bn: "সার্ভার টুলস" },
      category: "tools",
      core: true,
      permissions: ["admin.server.manage", "admin.nodes.manage"],
      features: ["server-tools"],
    },
    adminNav: [
      {
        id: "admin-server-tools",
        labelKey: "serverTools.nav",
        icon: <Wrench className="h-4 w-4" />,
        href: "/admin/server-tools",
        order: 64,
        permission: "admin.server.manage",
      },
    ],
  },
  {
    manifest: {
      name: "waf",
      title: { en: "WAF & Security", bn: "ডাব্লিউএএফ ও নিরাপত্তা" },
      category: "security",
      core: true,
      permissions: ["waf.read", "waf.manage", "admin.waf.manage"],
      features: ["waf"],
    },
    clientNav: [
      {
        id: "waf",
        labelKey: "waf.nav",
        icon: <ShieldAlert className="h-4 w-4" />,
        href: "/waf",
        order: 37,
        permission: "waf.read",
        feature: "waf",
      },
    ],
    adminNav: [
      {
        id: "admin-waf",
        labelKey: "waf.nav",
        icon: <ShieldAlert className="h-4 w-4" />,
        href: "/admin/waf",
        order: 63,
        permission: "admin.waf.manage",
      },
    ],
  },
  {
    manifest: {
      name: "firewall",
      title: { en: "Firewall & IP Block", bn: "ফায়ারওয়াল ও আইপি ব্লক" },
      category: "security",
      core: true,
      permissions: ["firewall.read", "firewall.manage", "admin.firewall.manage"],
      features: ["firewall"],
    },
    clientNav: [
      {
        id: "firewall",
        labelKey: "firewall.nav",
        icon: <Flame className="h-4 w-4" />,
        href: "/firewall",
        order: 38,
        permission: "firewall.read",
        feature: "firewall",
      },
    ],
    adminNav: [
      {
        id: "admin-firewall",
        labelKey: "firewall.nav",
        icon: <Flame className="h-4 w-4" />,
        href: "/admin/firewall",
        order: 65,
        permission: "admin.firewall.manage",
      },
    ],
  },
  {
    manifest: {
      name: "security_center",
      title: { en: "Security Center", bn: "নিরাপত্তা কেন্দ্র" },
      category: "core",
      core: true,
      permissions: ["admin.security.read"],
    },
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "apps",
      title: { en: "Applications", bn: "অ্যাপ্লিকেশনসমূহ" },
      category: "core",
      core: true,
      permissions: ["apps.read", "apps.install", "apps.manage"],
      features: ["apps"],
    },
    clientNav: [
      {
        id: "apps",
        labelKey: "apps.nav",
        icon: <Boxes className="h-4 w-4" />,
        href: "/apps",
        order: 30,
        permission: "apps.read",
      },
    ],
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "email",
      title: { en: "Email", bn: "ইমেইল" },
      category: "core",
      core: true,
      permissions: ["email.read", "email.manage", "admin.email.manage"],
      features: ["email"],
    },
    clientNav: [
      {
        id: "email",
        labelKey: "email.nav",
        icon: <Mail className="h-4 w-4" />,
        href: "/email",
        order: 35,
        permission: "email.read",
      },
    ],
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "resellers",
      title: { en: "Resellers", bn: "রিসেলার" },
      category: "core",
      core: true,
      permissions: ["resellers.read", "resellers.manage", "admin.resellers.manage"],
      features: ["resellers"],
    },
    clientNav: [
      {
        id: "reseller-portal",
        labelKey: "reseller.nav",
        icon: <Briefcase className="h-4 w-4" />,
        href: "/reseller",
        order: 38,
        permission: "resellers.read",
      },
    ],
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "migration",
      title: { en: "Migration & Transfers", bn: "মাইগ্রেশন ও ট্রান্সফার" },
      category: "core",
      core: true,
      permissions: ["migration.read", "migration.execute", "admin.migration.manage"],
      features: ["migration"],
    },
    adminNav: [
      {
        id: "admin-migration",
        labelKey: "migration.adminNav",
        icon: <ArrowRightLeft className="h-4 w-4" />,
        href: "/admin/migration",
        order: 60,
        permission: "admin.migration.manage",
      },
    ],
    clientNav: [
      {
        id: "client-migration",
        labelKey: "migration.nav",
        icon: <ArrowRightLeft className="h-4 w-4" />,
        href: "/migration",
        order: 42,
        permission: "migration.read",
      },
    ],
  },
  {
    manifest: {
      name: "settings",
      title: { en: "Settings", bn: "সেটিংস" },
      category: "core",
      core: true,
      permissions: ["admin.settings.read", "admin.settings.manage", "settings.read", "settings.update"],
    },
    adminNav: [
      {
        id: "settings",
        labelKey: "common.settings",
        icon: <Settings className="h-4 w-4" />,
        href: "/settings",
        order: 80,
        permission: "admin.settings.read",
      },
    ],
  },
  {
    manifest: {
      name: "license",
      title: { en: "License Authority", bn: "লাইসেন্স অথরিটি" },
      category: "core",
      core: true,
      permissions: ["admin.settings.manage"],
    },
    adminNav: [
      {
        id: "license",
        labelKey: "license.nav",
        icon: <KeyRound className="h-4 w-4" />,
        href: "/admin/license",
        order: 68,
        permission: "admin.settings.manage",
      },
    ],
  },
  {
    manifest: {
      name: "modules",
      title: { en: "Modules", bn: "মডিউলসমূহ" },
      category: "core",
      core: true,
      permissions: ["admin.modules.read"],
    },
    adminNav: [
      {
        id: "modules",
        labelKey: "modules.nav",
        icon: <Boxes className="h-4 w-4" />,
        href: "/modules",
        order: 70,
        permission: "admin.modules.read",
      },
    ],
  },
  {
    manifest: {
      name: "license-authority",
      title: { en: "License Authority", bn: "লাইসেন্স অথরিটি" },
      category: "core",
      core: true,
      permissions: ["admin.license.read", "admin.settings.read"],
    },
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "branding",
      title: { en: "Branding", bn: "ব্র্যান্ডিং" },
      category: "core",
      core: true,
      permissions: ["admin.branding.manage", "admin.settings.manage", "*"],
    },
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "update",
      title: { en: "Updates", bn: "আপডেট" },
      category: "core",
      core: true,
      permissions: ["admin.update.manage"],
    },
    adminNav: [
      {
        id: "update",
        labelKey: "update.nav",
        icon: <ArrowUpCircle className="h-4 w-4" />,
        href: "/update",
        order: 97,
        permission: "admin.settings.read",
      },
    ],
  },
  {
    manifest: {
      name: "api_tokens",
      title: { en: "API Tokens", bn: "এপিআই টোকেন" },
      category: "core",
      core: true,
      permissions: ["admin.tokens.manage"],
    },
    adminNav: [
    ],
  },
  {
    manifest: {
      name: "support_report",
      title: { en: "Support Report", bn: "সাপোর্ট রিপোর্ট" },
      category: "core",
      core: true,
      permissions: ["admin.support.read", "admin.settings.manage", "*"],
    },
    adminNav: [
    ],
  },
];

export const registeredModules: ModuleManifest[] = registeredFrontendModules.map((m) => m.manifest);

export interface FilterNavSession {
  roles?: string[];
  permissions?: string[];
  modules?: string[];
  features?: string[];
}

export function filterNavItems(
  modules: FrontendModule[],
  session: FilterNavSession | null | undefined,
  target: "admin" | "client" = "admin"
): ModuleNavItem[] {
  if (!session) return [];

  const activeModules = session.modules || [];
  // Some interface modules are parts of one backend module.
  const backendModuleOf = (name: string): string => (name === "dns" || name === "ssl" ? "domains" : name);
  const activeFeatures = session.features || [];
  const permissions = session.permissions || [];
  const roles = session.roles || [];

  const hasPerm = (perm: string | undefined): boolean => {
    if (!perm) return true;
    if (roles.includes("super_admin") || permissions.includes("*")) return true;
    if (permissions.includes(perm)) return true;
    return permissions.some((p) => {
      if (p.endsWith(".*")) {
        const prefix = p.slice(0, -2);
        return perm === prefix || perm.startsWith(`${prefix}.`);
      }
      return false;
    });
  };

  const hasFeat = (feat: string | undefined): boolean => {
    if (!feat) return true;
    if (activeFeatures.includes("all")) return true;
    return activeFeatures.includes(feat);
  };

  const navs: ModuleNavItem[] = [];

  for (const mod of modules) {
    // 1. Module Gate: Check if module is core or in active modules
    const isModuleEnabled =
      !session.modules || mod.manifest.core || activeModules.includes(backendModuleOf(mod.manifest.name));
    if (!isModuleEnabled) continue;

    // Pick target nav list
    const items = target === "admin" ? mod.adminNav : mod.clientNav;
    if (!items) continue;

    for (const item of items) {
      // 2. Feature Gate
      if (!hasFeat(item.feature)) continue;

      // 3. Permission Gate
      if (!hasPerm(item.permission)) continue;

      navs.push(item);
    }
  }

  // Sort by order
  return navs.sort((a, b) => (a.order || 100) - (b.order || 100));
}

export function getEnabledModules(activeModuleNames: string[]): ModuleManifest[] {
  return registeredModules.filter(
    (mod) => mod.core || activeModuleNames.includes(mod.name)
  );
}
