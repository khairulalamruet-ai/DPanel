import React, { useState, useEffect, useCallback, useMemo } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Input,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Server,
  Cpu,
  Activity,
  AlertTriangle,
  RefreshCw,
  Search,
  Power,
  XCircle,
  Play,
  Pause,
  ShieldAlert,
  Users,
} from "lucide-react";
import { ProcessItem, ProcessOverviewStats, ProcessSignal } from "./types";
import { KillProcessDialog } from "./KillProcessDialog";

export function AdminProcessesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [processes, setProcesses] = useState<ProcessItem[]>([]);
  const [stats, setStats] = useState<ProcessOverviewStats | null>(null);
  const [node, setNode] = useState<string>("node-01");
  const [tenantFilter, setTenantFilter] = useState<string>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [sortBy, setSortBy] = useState<"cpu" | "mem" | "pid">("cpu");
  const [isAutoRefresh, setIsAutoRefresh] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  // Kill Dialog State
  const [selectedProcess, setSelectedProcess] = useState<ProcessItem | null>(null);
  const [selectedSignal, setSelectedSignal] = useState<ProcessSignal>("SIGTERM");
  const [isKillOpen, setIsKillOpen] = useState(false);

  const fetchProcesses = useCallback(async () => {
    try {
      setIsLoading(true);
      const [listRes, statsRes] = await Promise.all([
        apiFetch<ProcessItem[]>(
          `/api/v1/admin/processes?node=${encodeURIComponent(node)}&tenant=${encodeURIComponent(tenantFilter)}`
        ),
        apiFetch<ProcessOverviewStats>(
          `/api/v1/admin/processes/stats?node=${encodeURIComponent(node)}`
        ),
      ]);
      setProcesses(listRes);
      setStats(statsRes);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [node, tenantFilter, t, toast]);

  useEffect(() => {
    fetchProcesses();
  }, [fetchProcesses]);

  useEffect(() => {
    if (!isAutoRefresh) return;
    const interval = setInterval(() => {
      fetchProcesses();
    }, 3000);
    return () => clearInterval(interval);
  }, [isAutoRefresh, fetchProcesses]);

  const handleOpenKill = (proc: ProcessItem, sig: ProcessSignal) => {
    setSelectedProcess(proc);
    setSelectedSignal(sig);
    setIsKillOpen(true);
  };

  const handleExecuteKill = async (pid: number, signal: ProcessSignal, sudoPassword?: string) => {
    await apiFetch(`/api/v1/admin/processes/${encodeURIComponent(pid)}/kill`, {
      method: "POST",
      body: JSON.stringify({ pid, signal, sudo_password: sudoPassword }),
    });

    toast({
      title: t("common.success"),
      description: t("processes.signalSentSuccess", { pid, signal }),
    });

    await fetchProcesses();
  };

  const filteredAndSorted = useMemo(() => {
    const list = processes.filter((p) => {
      if (!searchQuery.trim()) return true;
      const q = searchQuery.toLowerCase();
      return (
        p.pid.toString().includes(q) ||
        p.user.toLowerCase().includes(q) ||
        p.command.toLowerCase().includes(q) ||
        (p.tenant && p.tenant.toLowerCase().includes(q))
      );
    });

    list.sort((a, b) => {
      if (sortBy === "cpu") return b.cpu_percent - a.cpu_percent;
      if (sortBy === "mem") return b.mem_rss_mb - a.mem_rss_mb;
      return a.pid - b.pid;
    });

    return list;
  }, [processes, searchQuery, sortBy]);

  return (
    <div className="p-6 space-y-6" data-testid="admin-processes-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-processes-title">
              {t("adminProcesses.title")}
            </h1>
            <Badge variant="outline" className="border-amber-400 text-amber-600 dark:text-amber-400 gap-1 text-[11px]">
              <ShieldAlert className="w-3 h-3" />
              {t("adminProcesses.sudoModeActive")}
            </Badge>
          </div>
          <p className="text-sm text-muted-foreground">
            {t("adminProcesses.description")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Node Selector */}
          <select
            value={node}
            onChange={(e) => setNode(e.target.value)}
            className="h-8 text-xs rounded-md border border-input bg-background px-2.5 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            data-testid="select-admin-proc-node"
          >
            <option value="node-01">node-01 (master)</option>
            <option value="node-02">node-02 (worker)</option>
          </select>

          {/* Tenant Filter */}
          <select
            value={tenantFilter}
            onChange={(e) => setTenantFilter(e.target.value)}
            className="h-8 text-xs rounded-md border border-input bg-background px-2.5 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            data-testid="select-admin-proc-tenant"
          >
            <option value="all">{t("adminProcesses.allTenantsAndHost")}</option>
            <option value="host">{t("adminProcesses.hostSystemOnly")}</option>
            <option value="demo">demo</option>
            <option value="acme">acme</option>
          </select>

          {/* Auto Refresh */}
          <Button
            size="sm"
            variant={isAutoRefresh ? "default" : "outline"}
            onClick={() => setIsAutoRefresh(!isAutoRefresh)}
            className={`gap-1.5 text-xs ${isAutoRefresh ? "bg-green-600 hover:bg-green-700 text-white" : ""}`}
            data-testid="btn-toggle-admin-auto-refresh"
          >
            {isAutoRefresh ? (
              <>
                <span className="w-1.5 h-1.5 rounded-full bg-white animate-pulse"></span>
                <Pause className="w-3 h-3" />
                {t("processes.autoRefreshActive")}
              </>
            ) : (
              <>
                <Play className="w-3 h-3 text-green-500" />
                {t("processes.startAutoRefresh")}
              </>
            )}
          </Button>

          {/* Manual Refresh */}
          <Button
            size="sm"
            variant="outline"
            onClick={fetchProcesses}
            disabled={isLoading}
            className="gap-1.5 text-xs"
            data-testid="btn-refresh-admin-processes"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {/* Stats Cards */}
      {stats && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <Card data-testid="card-admin-proc-total">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminProcesses.nodeProcesses")}
              </CardTitle>
              <Server className="w-4 h-4 text-muted-foreground" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold" data-testid="metric-admin-total-procs">
                {stats.total_processes}
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {stats.running_processes} running, {stats.sleeping_processes} sleeping
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-proc-cpu">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminProcesses.hostCpuLoad")}
              </CardTitle>
              <Cpu className="w-4 h-4 text-blue-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-blue-600 dark:text-blue-400" data-testid="metric-admin-total-cpu">
                {stats.total_cpu_percent}%
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                Across 32 system cores
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-proc-ram">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("adminProcesses.hostMemoryRss")}
              </CardTitle>
              <Activity className="w-4 h-4 text-emerald-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-emerald-600 dark:text-emerald-400" data-testid="metric-admin-total-ram">
                {stats.total_mem_rss_mb} MB
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                Resident memory allocated
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-admin-proc-zombie">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("processes.metricZombies")}
              </CardTitle>
              <AlertTriangle className="w-4 h-4 text-muted-foreground" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold" data-testid="metric-admin-total-zombies">
                {stats.zombie_processes}
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                Defunct processes
              </p>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Processes Table */}
      <Card>
        <CardHeader className="p-4 border-b">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <CardTitle className="text-base font-semibold">
              {t("adminProcesses.clusterProcessesTitle")}
            </CardTitle>

            <div className="flex items-center gap-2">
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span>{t("common.sortBy")}:</span>
                <select
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value as any)}
                  className="h-8 text-xs rounded-md border border-input bg-background px-2 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                >
                  <option value="cpu">% CPU</option>
                  <option value="mem">% RSS</option>
                  <option value="pid">PID</option>
                </select>
              </div>

              <div className="relative w-48 sm:w-60">
                <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder={t("processes.searchPlaceholder")}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="pl-8 text-xs h-8"
                  data-testid="input-admin-search-procs"
                />
              </div>
            </div>
          </div>
        </CardHeader>

        <CardContent className="p-0">
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[80px]">{t("processes.thPid")}</TableHead>
                  <TableHead className="w-[120px]">{t("adminProcesses.thScope")}</TableHead>
                  <TableHead className="w-[100px]">{t("processes.thUser")}</TableHead>
                  <TableHead className="w-[110px]">{t("processes.thCpu")}</TableHead>
                  <TableHead className="w-[120px]">{t("processes.thMem")}</TableHead>
                  <TableHead className="w-[100px]">{t("processes.thStat")}</TableHead>
                  <TableHead className="w-[90px]">{t("processes.thTime")}</TableHead>
                  <TableHead>{t("processes.thCommand")}</TableHead>
                  <TableHead className="text-right w-[160px]">{t("processes.thActions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredAndSorted.map((proc) => (
                  <TableRow key={proc.pid} data-testid={`admin-process-row-${proc.pid}`}>
                    <TableCell className="font-mono text-xs font-bold" data-testid={`admin-proc-pid-${proc.pid}`}>
                      {proc.pid}
                    </TableCell>

                    <TableCell>
                      {proc.tenant ? (
                        <Badge variant="secondary" className="font-mono text-[10px]">
                          {proc.tenant}
                        </Badge>
                      ) : (
                        <Badge variant="outline" className="font-mono text-[10px] text-muted-foreground">
                          host/system
                        </Badge>
                      )}
                    </TableCell>

                    <TableCell className="text-xs text-muted-foreground font-mono">
                      {proc.user}
                    </TableCell>

                    <TableCell className="font-mono text-xs font-semibold">
                      {proc.cpu_percent}%
                    </TableCell>

                    <TableCell className="font-mono text-xs font-semibold">
                      {proc.mem_rss_mb} MB
                    </TableCell>

                    <TableCell className="font-mono text-xs">
                      {proc.stat}
                    </TableCell>

                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {proc.cpu_time}
                    </TableCell>

                    <TableCell>
                      <div className="font-mono text-xs max-w-[340px] truncate text-foreground font-medium" data-testid={`admin-proc-cmd-${proc.pid}`}>
                        {proc.command}
                      </div>
                    </TableCell>

                    <TableCell className="text-right">
                      {proc.is_killable ? (
                        <div className="flex items-center justify-end gap-1.5">
                          <Button
                            size="sm"
                            variant="outline"
                            className="h-7 px-2 text-xs text-amber-600 border-amber-300"
                            onClick={() => handleOpenKill(proc, "SIGTERM")}
                            data-testid={`btn-admin-sigterm-${proc.pid}`}
                          >
                            SIGTERM
                          </Button>
                          <Button
                            size="sm"
                            variant="destructive"
                            className="h-7 px-2 text-xs"
                            onClick={() => handleOpenKill(proc, "SIGKILL")}
                            data-testid={`btn-admin-sigkill-${proc.pid}`}
                          >
                            SIGKILL
                          </Button>
                        </div>
                      ) : (
                        <span className="text-[11px] text-muted-foreground italic">
                          Protected
                        </span>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>

      {/* Sudo Kill Dialog */}
      <KillProcessDialog
        open={isKillOpen}
        onOpenChange={setIsKillOpen}
        process={selectedProcess}
        defaultSignal={selectedSignal}
        isSudoMode={true}
        onConfirm={handleExecuteKill}
      />
    </div>
  );
}
