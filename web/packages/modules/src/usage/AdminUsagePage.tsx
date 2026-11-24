import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { listNodeChoices, type NodeChoice } from "../nodes/api";
import { loadAdminUsage } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Server,
  Cpu,
  Activity,
  HardDrive,
  ArrowUpRight,
  RefreshCw,
  Users,
  AlertTriangle,
  TrendingUp,
} from "lucide-react";
import { AdminUsageOverview, TimeRange, TopTenantUsageItem } from "./types";
import { UsageAreaChart } from "./UsageAreaChart";

export function AdminUsagePage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const [node, setNode] = useState<string>("");
  const [range, setRange] = useState<TimeRange>("24h");
  const [data, setData] = useState<AdminUsageOverview | null>(null);
  const [activeTopTab, setActiveTopTab] = useState<"cpu" | "ram" | "disk" | "bandwidth">("cpu");
  const [isLoading, setIsLoading] = useState(false);

  useEffect(() => {
    listNodeChoices()
      .then((list) => {
        setNodes(list);
        setNode((current) => current || list[0]?.id || "");
      })
      .catch(() => undefined);
  }, []);

  const fetchAdminUsage = useCallback(async () => {
    if (!node) return;
    try {
      setIsLoading(true);
      const res = await loadAdminUsage(node, nodes.find((n) => n.id === node)?.name ?? "", range);
      setData(res);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [node, nodes, range, t, toast]);

  useEffect(() => {
    fetchAdminUsage();
  }, [fetchAdminUsage]);

  const ranges: { id: TimeRange; label: string }[] = [
    { id: "24h", label: t("usage.range24h") },
    { id: "7d", label: t("usage.range7d") },
    { id: "30d", label: t("usage.range30d") },
  ];

  const currentTopList: TopTenantUsageItem[] = data
    ? activeTopTab === "cpu"
      ? data.top_tenants_cpu
      : activeTopTab === "ram"
      ? data.top_tenants_ram
      : activeTopTab === "disk"
      ? data.top_tenants_disk
      : data.top_tenants_bandwidth
    : [];

  return (
    <div className="p-6 space-y-6" data-testid="admin-usage-page">
      {/* Header & Controls */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-usage-title">
            {t("adminUsage.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("adminUsage.description")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Node Selector */}
          <select
            value={node}
            onChange={(e) => setNode(e.target.value)}
            className="h-8 text-xs rounded-md border border-input bg-background px-2.5 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            data-testid="select-admin-usage-node"
          >
            {nodes.map((n) => (
              <option key={n.id} value={n.id}>
                {n.name}
              </option>
            ))}
          </select>

          {/* Range Selector */}
          <div className="flex items-center rounded-lg bg-muted p-1 text-xs">
            {ranges.map((r) => (
              <button
                key={r.id}
                type="button"
                onClick={() => setRange(r.id)}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  range === r.id
                    ? "bg-background text-foreground shadow-sm font-semibold"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid={`btn-admin-range-${r.id}`}
              >
                {r.label}
              </button>
            ))}
          </div>

          {/* Refresh */}
          <Button
            size="sm"
            variant="ghost"
            onClick={fetchAdminUsage}
            disabled={isLoading}
            className="h-8 w-8 p-0"
            data-testid="btn-refresh-admin-usage"
            title={t("common.refresh")}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {/* Node Summary Metrics */}
      {data && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
          <Card data-testid="card-admin-node-cpu">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminUsage.nodeCpu")}
              </CardTitle>
              <Cpu className="w-4 h-4 text-blue-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-admin-node-cpu">
                  {data.node_cpu.current}%
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  32 Cores ({data.node_cpu.limit} cores)
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-blue-600 dark:bg-blue-500 h-2 rounded-full"
                  style={{ width: `${Math.min(100, data.node_cpu.current)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.node_cpu.average}%</span>
                <span>Peak: {data.node_cpu.peak}%</span>
              </div>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-node-ram">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminUsage.nodeRam")}
              </CardTitle>
              <Activity className="w-4 h-4 text-emerald-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-admin-node-ram">
                  {data.node_ram.current} GB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  of {data.node_ram.limit} GB Total
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-emerald-600 dark:bg-emerald-500 h-2 rounded-full"
                  style={{ width: `${Math.min(100, (data.node_ram.current / (data.node_ram.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.node_ram.average} GB</span>
                <span>Peak: {data.node_ram.peak} GB</span>
              </div>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-node-disk">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminUsage.nodeDisk")}
              </CardTitle>
              <HardDrive className="w-4 h-4 text-amber-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-admin-node-disk">
                  {data.node_disk.current} GB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  of {data.node_disk.limit} GB ZFS Pool
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-amber-600 dark:bg-amber-500 h-2 rounded-full"
                  style={{ width: `${Math.min(100, (data.node_disk.current / (data.node_disk.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.node_disk.average} GB</span>
                <span>Peak: {data.node_disk.peak} GB</span>
              </div>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-node-network">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminUsage.nodeNetwork")}
              </CardTitle>
              <ArrowUpRight className="w-4 h-4 text-purple-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-admin-node-network">
                  {data.node_bandwidth.current} GB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  10 Gbps Uplink
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-purple-600 dark:bg-purple-500 h-2 rounded-full"
                  style={{ width: `${Math.min(100, (data.node_bandwidth.current / (data.node_bandwidth.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.node_bandwidth.average} GB</span>
                <span>Peak: {data.node_bandwidth.peak} GB</span>
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Node History Line Chart */}
      {data && (
        <Card data-testid="card-admin-history-chart">
          <CardHeader className="pb-2 border-b">
            <div className="flex items-center justify-between">
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <TrendingUp className="w-4 h-4 text-primary" />
                {t("adminUsage.nodeHistoryTitle")} ({data.node_name})
              </CardTitle>
              <Badge variant="outline" className="font-mono text-xs">
                Range: {range}
              </Badge>
            </div>
          </CardHeader>
          <CardContent className="pt-4">
            <UsageAreaChart
              data={data.history}
              dataKey="cpu_percent"
              unit="%"
              color="primary"
              height={220}
              maxVal={100}
            />
          </CardContent>
        </Card>
      )}

      {/* Top-N Tenants Section */}
      <Card data-testid="card-top-tenants">
        <CardHeader className="p-4 border-b">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <div className="flex items-center gap-2">
              <Users className="w-4 h-4 text-primary" />
              <CardTitle className="text-base font-semibold">
                {t("adminUsage.topTenantsTitle")}
              </CardTitle>
            </div>

            {/* Top-N metric tabs */}
            <div className="flex items-center rounded-lg bg-muted p-1 text-xs">
              <button
                type="button"
                onClick={() => setActiveTopTab("cpu")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  activeTopTab === "cpu"
                    ? "bg-background text-foreground shadow-sm font-semibold"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-top-cpu"
              >
                {t("adminUsage.topCpu")}
              </button>
              <button
                type="button"
                onClick={() => setActiveTopTab("ram")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  activeTopTab === "ram"
                    ? "bg-background text-foreground shadow-sm font-semibold"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-top-ram"
              >
                {t("adminUsage.topRam")}
              </button>
              <button
                type="button"
                onClick={() => setActiveTopTab("disk")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  activeTopTab === "disk"
                    ? "bg-background text-foreground shadow-sm font-semibold"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-top-disk"
              >
                {t("adminUsage.topDisk")}
              </button>
              <button
                type="button"
                onClick={() => setActiveTopTab("bandwidth")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  activeTopTab === "bandwidth"
                    ? "bg-background text-foreground shadow-sm font-semibold"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-top-bandwidth"
              >
                {t("adminUsage.topBandwidth")}
              </button>
            </div>
          </div>
        </CardHeader>

        <CardContent className="p-0">
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("adminUsage.thTenant")}</TableHead>
                  <TableHead>{t("adminUsage.thPlan")}</TableHead>
                  <TableHead>{t("adminUsage.thUsageMetric")}</TableHead>
                  <TableHead>{t("adminUsage.thAllocation")}</TableHead>
                  <TableHead>{t("adminUsage.thStatus")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {currentTopList.map((tenant, idx) => {
                  const usageVal =
                    activeTopTab === "cpu"
                      ? `${tenant.cpu_percent}%`
                      : activeTopTab === "ram"
                      ? `${tenant.ram_mb} MB`
                      : activeTopTab === "disk"
                      ? `${tenant.disk_gb} GB`
                      : `${tenant.bandwidth_30d_gb} GB`;

                  const limitVal =
                    activeTopTab === "cpu"
                      ? "100%"
                      : activeTopTab === "ram"
                      ? `${tenant.ram_limit_mb} MB`
                      : activeTopTab === "disk"
                      ? `${tenant.disk_quota_gb} GB`
                      : "500 GB";

                  const pct =
                    activeTopTab === "cpu"
                      ? tenant.cpu_percent
                      : activeTopTab === "ram"
                      ? Math.round((tenant.ram_mb / (tenant.ram_limit_mb || 1)) * 100)
                      : activeTopTab === "disk"
                      ? Math.round((tenant.disk_gb / (tenant.disk_quota_gb || 1)) * 100)
                      : Math.round((tenant.bandwidth_30d_gb / 500) * 100);

                  return (
                    <TableRow key={tenant.tenant_id} data-testid={`row-tenant-${tenant.tenant_id}`}>
                      <TableCell>
                        <div className="font-semibold text-sm" data-testid={`tenant-name-${tenant.tenant_id}`}>
                          {tenant.tenant_name}
                        </div>
                        <div className="text-[11px] text-muted-foreground font-mono">
                          ID: {tenant.tenant_id}
                        </div>
                      </TableCell>

                      <TableCell>
                        <Badge variant="outline" className="text-xs">
                          {tenant.plan}
                        </Badge>
                      </TableCell>

                      <TableCell>
                        <div className="space-y-1 w-48">
                          <div className="flex items-center justify-between text-xs font-semibold">
                            <span>{usageVal}</span>
                            <span className="text-muted-foreground font-normal">{pct}%</span>
                          </div>
                          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
                            <div
                              className={`h-1.5 rounded-full ${
                                pct > 85 ? "bg-red-500" : pct > 65 ? "bg-amber-500" : "bg-primary"
                              }`}
                              style={{ width: `${Math.min(100, pct)}%` }}
                            />
                          </div>
                        </div>
                      </TableCell>

                      <TableCell className="text-xs font-mono text-muted-foreground">
                        {limitVal}
                      </TableCell>

                      <TableCell>
                        {tenant.status === "critical" ? (
                          <Badge variant="destructive" className="text-[10px] gap-1">
                            <AlertTriangle className="w-3 h-3" />
                            {t("adminUsage.statusCritical")}
                          </Badge>
                        ) : tenant.status === "warning" ? (
                          <Badge variant="outline" className="text-amber-600 bg-amber-500/10 border-amber-400 text-[10px]">
                            {t("adminUsage.statusWarning")}
                          </Badge>
                        ) : (
                          <Badge variant="outline" className="text-green-600 bg-green-500/10 border-green-400 text-[10px]">
                            {t("adminUsage.statusNormal")}
                          </Badge>
                        )}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
