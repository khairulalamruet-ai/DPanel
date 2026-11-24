import React from "react";
import { useTranslation } from "@builtpanel/i18n";
import { Card, CardHeader, CardTitle, CardContent, Badge, Button } from "@builtpanel/ui";
import {
  Server,
  Users,
  Activity,
  History,
  ShieldAlert,
  KeyRound,
  CheckCircle2,
  AlertTriangle,
  ArrowRight,
  HardDrive,
  Cpu,
  Globe,
  Database,
  ExternalLink,
} from "lucide-react";
import { AdminDashboardSummary } from "./types";

interface WidgetProps {
  summary: AdminDashboardSummary;
}

export function NodesStatusWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { nodes } = summary;
  const isHealthy = nodes.online === nodes.total;

  return (
    <Card className="border shadow-xs" data-testid="widget-nodes_status">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <Server className="h-4 w-4 text-primary" />
          <span>{t("dashboard.widgetNodes", "Cluster Nodes Status")}</span>
        </CardTitle>
        <Badge variant={isHealthy ? "success" : "destructive"} className="text-[10px] font-mono">
          {isHealthy ? "HEALTHY" : "DEGRADED"}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-baseline justify-between">
          <div className="space-y-0.5">
            <span className="text-2xl font-bold font-mono" data-testid="stat-nodes-online">
              {nodes.online} / {nodes.total}
            </span>
            <span className="text-xs text-muted-foreground block">
              {t("dashboard.nodesOnlineLabel", "Nodes Active & Heartbeat OK")}
            </span>
          </div>
          <div className="text-right text-xs font-mono space-y-0.5">
            <div>
              <span className="text-muted-foreground">Avg CPU: </span>
              <span className="font-bold">{nodes.avg_cpu_percent}%</span>
            </div>
            <div>
              <span className="text-muted-foreground">Avg RAM: </span>
              <span className="font-bold">{nodes.avg_ram_percent}%</span>
            </div>
          </div>
        </div>

        <div className="pt-2 border-t flex justify-end">
          <a
            href="/nodes"
            className="text-[11px] text-primary hover:underline font-semibold flex items-center gap-1"
          >
            <span>{t("dashboard.viewNodes", "View Nodes")}</span>
            <ArrowRight className="h-3 w-3" />
          </a>
        </div>
      </CardContent>
    </Card>
  );
}

export function TenantsCountWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { tenants } = summary;

  return (
    <Card className="border shadow-xs" data-testid="widget-tenants_count">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <Users className="h-4 w-4 text-emerald-500" />
          <span>{t("dashboard.widgetTenants", "Tenants & Hosted Domains")}</span>
        </CardTitle>
        <Badge variant="outline" className="text-[10px] font-mono">
          {tenants.active} ACTIVE
        </Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-3 gap-2 text-center pt-1">
          <div className="p-2 rounded-lg bg-muted/20">
            <span className="text-xl font-bold font-mono block" data-testid="stat-tenants-count">
              {tenants.total}
            </span>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold">Tenants</span>
          </div>
          <div className="p-2 rounded-lg bg-muted/20">
            <span className="text-xl font-bold font-mono block" data-testid="stat-domains-count">
              {tenants.domains_count}
            </span>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold">Domains</span>
          </div>
          <div className="p-2 rounded-lg bg-muted/20">
            <span className="text-xl font-bold font-mono block" data-testid="stat-databases-count">
              {tenants.databases_count}
            </span>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold">Databases</span>
          </div>
        </div>

        <div className="pt-2 border-t flex justify-end">
          <a
            href="/users"
            className="text-[11px] text-primary hover:underline font-semibold flex items-center gap-1"
          >
            <span>{t("dashboard.manageTenants", "Manage Tenants")}</span>
            <ArrowRight className="h-3 w-3" />
          </a>
        </div>
      </CardContent>
    </Card>
  );
}

export function ResourceUsageWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { resources } = summary;
  const ramPercent = Math.round((resources.memory_used_mb / resources.memory_total_mb) * 100);
  const diskPercent = Math.round((resources.disk_used_gb / resources.disk_total_gb) * 100);

  return (
    <Card className="border shadow-xs" data-testid="widget-resource_usage">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <Activity className="h-4 w-4 text-primary" />
          <span>{t("dashboard.widgetResources", "Resource Consumption")}</span>
        </CardTitle>
        <span className="text-[11px] text-muted-foreground font-mono">
          Rx: {resources.network_rx_mb} MB/s
        </span>
      </CardHeader>
      <CardContent className="space-y-3">
        {/* CPU */}
        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs font-mono">
            <span>CPU Usage</span>
            <span className="font-bold">{resources.cpu_percent}%</span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full ${resources.cpu_percent > 80 ? "bg-destructive" : "bg-primary"}`}
              style={{ width: `${resources.cpu_percent}%` }}
            />
          </div>
        </div>

        {/* RAM */}
        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs font-mono">
            <span>Memory (RAM)</span>
            <span className="font-bold">
              {Math.round(resources.memory_used_mb / 1024 * 10) / 10} / {Math.round(resources.memory_total_mb / 1024)} GB
            </span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full ${ramPercent > 85 ? "bg-destructive" : "bg-primary"}`}
              style={{ width: `${ramPercent}%` }}
            />
          </div>
        </div>

        {/* Disk */}
        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs font-mono">
            <span>Root Storage</span>
            <span className="font-bold">
              {resources.disk_used_gb} / {resources.disk_total_gb} GB
            </span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full ${diskPercent > 90 ? "bg-destructive" : "bg-primary"}`}
              style={{ width: `${diskPercent}%` }}
            />
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function RecentOperationsWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { operations } = summary;

  return (
    <Card className="border shadow-xs" data-testid="widget-recent_operations">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <History className="h-4 w-4 text-blue-500" />
          <span>{t("dashboard.widgetOperations", "Recent Operations")}</span>
        </CardTitle>
        <a
          href="/operations"
          className="text-[11px] text-primary hover:underline font-semibold flex items-center gap-1"
        >
          <span>Ledger</span>
          <ArrowRight className="h-3 w-3" />
        </a>
      </CardHeader>
      <CardContent className="space-y-2">
        <div className="space-y-2 max-h-36 overflow-y-auto divide-y divide-border/40">
          {operations.map((op) => (
            <div key={op.id} className="pt-1.5 first:pt-0 flex items-center justify-between text-xs">
              <div className="space-y-0.5 truncate pr-2">
                <span className="font-mono font-bold block truncate">{op.type}</span>
                <span className="text-[10px] text-muted-foreground font-mono">{op.id}</span>
              </div>
              <div className="flex items-center gap-1.5 shrink-0">
                {op.duration_ms && (
                  <span className="text-[10px] text-muted-foreground font-mono">
                    {op.duration_ms}ms
                  </span>
                )}
                <Badge
                  variant={op.status === "completed" ? "success" : op.status === "failed" ? "destructive" : "secondary"}
                  className="text-[10px] uppercase font-mono px-1.5 py-0"
                >
                  {op.status}
                </Badge>
              </div>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

export function SecurityAlertsWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { security } = summary;

  return (
    <Card className="border shadow-xs" data-testid="widget-security_alerts">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <ShieldAlert className="h-4 w-4 text-amber-500" />
          <span>{t("dashboard.widgetSecurity", "Security & Threats")}</span>
        </CardTitle>
        <Badge variant="outline" className="text-[10px] font-mono text-emerald-600 border-emerald-500/40">
          WAF ACTIVE
        </Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-baseline justify-between">
          <div>
            <span className="text-2xl font-bold font-mono text-foreground" data-testid="stat-blocked-threats">
              {security.blocked_threats_24h}
            </span>
            <span className="text-xs text-muted-foreground block">
              {t("dashboard.threatsBlocked24h", "Threats Blocked (24h)")}
            </span>
          </div>
          <div className="text-right text-xs font-mono space-y-0.5">
            <div>
              <span className="text-muted-foreground">Fail2ban Jails: </span>
              <span className="font-bold">{security.ssh_bans} IP banned</span>
            </div>
            <div>
              <span className="text-muted-foreground">Rules Enforced: </span>
              <span className="font-bold">{security.active_waf_rules} CRS rules</span>
            </div>
          </div>
        </div>

        <div className="pt-2 border-t flex justify-end">
          <a
            href="/waf"
            className="text-[11px] text-primary hover:underline font-semibold flex items-center gap-1"
          >
            <span>{t("dashboard.inspectWaf", "WAF Console")}</span>
            <ArrowRight className="h-3 w-3" />
          </a>
        </div>
      </CardContent>
    </Card>
  );
}

export function LicenseStatusWidget({ summary }: WidgetProps) {
  const { t } = useTranslation();
  const { license } = summary;

  return (
    <Card className="border shadow-xs" data-testid="widget-license_status">
      <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-purple-500" />
          <span>{t("dashboard.widgetLicense", "License Authority")}</span>
        </CardTitle>
        <Badge variant="success" className="text-[10px] font-mono">
          {license.status.toUpperCase()}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-baseline justify-between">
          <div className="space-y-0.5">
            <span className="text-lg font-bold" data-testid="stat-license-tier">
              {license.tier}
            </span>
            <span className="text-xs text-emerald-600 dark:text-emerald-400 flex items-center gap-1 font-semibold">
              <CheckCircle2 className="h-3 w-3" />
              Ed25519 Verified
            </span>
          </div>

          <div className="text-right text-xs font-mono space-y-0.5">
            <span className="text-muted-foreground block">Expires In</span>
            <span className="font-bold text-primary" data-testid="stat-license-days">
              {license.expires_in_days} Days
            </span>
          </div>
        </div>

        <div className="pt-2 border-t flex justify-end">
          <a
            href="/admin/license"
            className="text-[11px] text-primary hover:underline font-semibold flex items-center gap-1"
          >
            <span>{t("dashboard.manageLicense", "Manage License")}</span>
            <ArrowRight className="h-3 w-3" />
          </a>
        </div>
      </CardContent>
    </Card>
  );
}
