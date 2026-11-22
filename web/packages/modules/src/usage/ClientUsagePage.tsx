import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { loadClientUsage } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Cpu,
  HardDrive,
  Activity,
  ArrowUpRight,
  ArrowDownLeft,
  RefreshCw,
  Layers,
  Sparkles,
} from "lucide-react";
import { ClientUsageSummary, TimeRange } from "./types";
import { UsageAreaChart } from "./UsageAreaChart";

export function ClientUsagePage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [range, setRange] = useState<TimeRange>("24h");
  const [data, setData] = useState<ClientUsageSummary | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const fetchUsage = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await loadClientUsage(range);
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
  }, [range, t, toast]);

  useEffect(() => {
    fetchUsage();
  }, [fetchUsage]);

  const ranges: { id: TimeRange; label: string }[] = [
    { id: "24h", label: t("usage.range24h") },
    { id: "7d", label: t("usage.range7d") },
    { id: "30d", label: t("usage.range30d") },
  ];

  return (
    <div className="p-6 space-y-6" data-testid="client-usage-page">
      {/* Header & Controls */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="usage-page-title">
            {t("usage.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("usage.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
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
                data-testid={`btn-range-${r.id}`}
              >
                {r.label}
              </button>
            ))}
          </div>

          {/* Refresh */}
          <Button
            size="sm"
            variant="ghost"
            onClick={fetchUsage}
            disabled={isLoading}
            className="h-8 w-8 p-0"
            data-testid="btn-refresh-usage"
            title={t("common.refresh")}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {/* Top 4 Resource Gauges / Summary Cards */}
      {data && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
          {/* CPU Card */}
          <Card data-testid="card-usage-cpu">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("usage.cpuMetric")}
              </CardTitle>
              <Cpu className="w-4 h-4 text-blue-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-current-cpu">
                  {data.cpu.current}%
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  Limit: {data.cpu.limit} cores
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-blue-600 dark:bg-blue-500 h-2 rounded-full transition-all duration-500"
                  style={{ width: `${Math.min(100, data.cpu.current)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.cpu.average}%</span>
                <span>Peak: {data.cpu.peak}%</span>
              </div>
            </CardContent>
          </Card>

          {/* RAM Card */}
          <Card data-testid="card-usage-ram">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("usage.ramMetric")}
              </CardTitle>
              <Activity className="w-4 h-4 text-emerald-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-current-ram">
                  {data.ram.current} MB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  {Math.round((data.ram.current / (data.ram.limit || 1)) * 100)}% of {data.ram.limit} MB
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-emerald-600 dark:bg-emerald-500 h-2 rounded-full transition-all duration-500"
                  style={{ width: `${Math.min(100, (data.ram.current / (data.ram.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.ram.average} MB</span>
                <span>Peak: {data.ram.peak} MB</span>
              </div>
            </CardContent>
          </Card>

          {/* Disk Storage Card */}
          <Card data-testid="card-usage-disk">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("usage.diskMetric")}
              </CardTitle>
              <HardDrive className="w-4 h-4 text-amber-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-current-disk">
                  {data.disk.current} GB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  {Math.round((data.disk.current / (data.disk.limit || 1)) * 100)}% of {data.disk.limit} GB
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-amber-600 dark:bg-amber-500 h-2 rounded-full transition-all duration-500"
                  style={{ width: `${Math.min(100, (data.disk.current / (data.disk.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.disk.average} GB</span>
                <span>Peak: {data.disk.peak} GB</span>
              </div>
            </CardContent>
          </Card>

          {/* Network Bandwidth Card */}
          <Card data-testid="card-usage-bandwidth">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("usage.bandwidthMetric")}
              </CardTitle>
              <ArrowUpRight className="w-4 h-4 text-purple-500" />
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-baseline justify-between">
                <span className="text-2xl font-bold" data-testid="metric-current-bandwidth">
                  {data.bandwidth.current} GB
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  Limit: {data.bandwidth.limit} GB
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
                <div
                  className="bg-purple-600 dark:bg-purple-500 h-2 rounded-full transition-all duration-500"
                  style={{ width: `${Math.min(100, (data.bandwidth.current / (data.bandwidth.limit || 1)) * 100)}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1">
                <span>Avg: {data.bandwidth.average} GB</span>
                <span>Peak: {data.bandwidth.peak} GB</span>
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Charts Grid */}
      {data && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* CPU Chart */}
          <Card data-testid="card-chart-cpu">
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <Cpu className="w-4 h-4 text-blue-500" />
                  {t("usage.chartCpuTitle")}
                </CardTitle>
                <Badge variant="outline" className="text-blue-600 dark:text-blue-400 font-mono text-xs">
                  Peak: {data.cpu.peak}%
                </Badge>
              </div>
            </CardHeader>
            <CardContent>
              <UsageAreaChart
                data={data.history}
                dataKey="cpu_percent"
                unit="%"
                color="primary"
                maxVal={100}
                height={200}
              />
            </CardContent>
          </Card>

          {/* RAM Chart */}
          <Card data-testid="card-chart-ram">
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <Activity className="w-4 h-4 text-emerald-500" />
                  {t("usage.chartRamTitle")}
                </CardTitle>
                <Badge variant="outline" className="text-emerald-600 dark:text-emerald-400 font-mono text-xs">
                  Peak: {data.ram.peak} MB
                </Badge>
              </div>
            </CardHeader>
            <CardContent>
              <UsageAreaChart
                data={data.history}
                dataKey="ram_mb"
                unit="MB"
                color="emerald"
                height={200}
              />
            </CardContent>
          </Card>

          {/* Storage Growth Chart */}
          <Card data-testid="card-chart-disk">
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <HardDrive className="w-4 h-4 text-amber-500" />
                  {t("usage.chartDiskTitle")}
                </CardTitle>
                <Badge variant="outline" className="text-amber-600 dark:text-amber-400 font-mono text-xs">
                  Quota: {data.disk.limit} GB
                </Badge>
              </div>
            </CardHeader>
            <CardContent>
              <UsageAreaChart
                data={data.history}
                dataKey="disk_gb"
                unit="GB"
                color="amber"
                height={200}
              />
            </CardContent>
          </Card>

          {/* Network I/O Chart */}
          <Card data-testid="card-chart-network">
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <ArrowUpRight className="w-4 h-4 text-purple-500" />
                  {t("usage.chartNetworkTitle")}
                </CardTitle>
                <div className="flex items-center gap-2 text-xs">
                  <span className="flex items-center gap-1 text-blue-500 font-mono">
                    <span className="w-2 h-2 rounded-full bg-blue-500"></span> Inbound
                  </span>
                  <span className="flex items-center gap-1 text-purple-500 font-mono">
                    <span className="w-2 h-2 rounded-full bg-purple-500"></span> Outbound
                  </span>
                </div>
              </div>
            </CardHeader>
            <CardContent>
              <UsageAreaChart
                data={data.history}
                dataKey="net_in_kbps"
                secondaryKey="net_out_kbps"
                unit="KB/s"
                color="primary"
                height={200}
              />
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  );
}
