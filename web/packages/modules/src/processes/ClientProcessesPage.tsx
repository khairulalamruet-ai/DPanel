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
import { listProcesses, signalProcess } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Terminal,
  Cpu,
  Activity,
  AlertTriangle,
  RefreshCw,
  Search,
  Power,
  XCircle,
  Play,
  Pause,
  Layers,
} from "lucide-react";
import { ProcessItem, ProcessOverviewStats, ProcessSignal } from "./types";
import { KillProcessDialog } from "./KillProcessDialog";

export function ClientProcessesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [processes, setProcesses] = useState<ProcessItem[]>([]);
  const [stats, setStats] = useState<ProcessOverviewStats | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [sortBy, setSortBy] = useState<"cpu" | "mem" | "pid">("cpu");
  const [isAutoRefresh, setIsAutoRefresh] = useState(false);

  // Kill Dialog State
  const [selectedProcess, setSelectedProcess] = useState<ProcessItem | null>(null);
  const [selectedSignal, setSelectedSignal] = useState<ProcessSignal>("SIGTERM");
  const [isKillOpen, setIsKillOpen] = useState(false);

  const fetchProcesses = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await listProcesses();
      setProcesses(res.items);
      setStats(res.stats);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchProcesses();
  }, [fetchProcesses]);

  // Auto-refresh interval (3s)
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

  const handleExecuteKill = async (pid: number, signal: ProcessSignal) => {
    const target = processes.find((p) => p.pid === pid);
    if (!target?.container) throw new Error("Unknown process");
    await signalProcess(target.container, pid, signal);

    toast({
      title: t("common.success"),
      description: t("processes.signalSentSuccess", { pid, signal }),
    });

    await fetchProcesses();
  };

  // Filter and sort processes
  const filteredAndSorted = useMemo(() => {
    const list = processes.filter((p) => {
      if (!searchQuery.trim()) return true;
      const q = searchQuery.toLowerCase();
      return (
        p.pid.toString().includes(q) ||
        p.user.toLowerCase().includes(q) ||
        p.command.toLowerCase().includes(q)
      );
    });

    list.sort((a, b) => {
      if (sortBy === "cpu") return b.cpu_percent - a.cpu_percent;
      if (sortBy === "mem") return b.mem_rss_mb - a.mem_rss_mb;
      return a.pid - b.pid;
    });

    return list;
  }, [processes, searchQuery, sortBy]);

  const formatStatBadge = (stat: string) => {
    if (stat.startsWith("R")) {
      return (
        <Badge variant="outline" className="text-green-600 bg-green-500/10 border-green-400 text-[10px]">
          {t("processes.statRunning")} ({stat})
        </Badge>
      );
    }
    if (stat.startsWith("Z")) {
      return (
        <Badge variant="destructive" className="text-[10px]">
          {t("processes.statZombie")} ({stat})
        </Badge>
      );
    }
    return (
      <Badge variant="outline" className="text-muted-foreground text-[10px]">
        {t("processes.statSleep")} ({stat})
      </Badge>
    );
  };

  return (
    <div className="p-6 space-y-6" data-testid="client-processes-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="processes-page-title">
            {t("processes.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("processes.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {/* Auto Refresh Toggle */}
          <Button
            size="sm"
            variant={isAutoRefresh ? "default" : "outline"}
            onClick={() => setIsAutoRefresh(!isAutoRefresh)}
            className={`gap-1.5 text-xs ${isAutoRefresh ? "bg-green-600 hover:bg-green-700 text-white" : ""}`}
            data-testid="btn-toggle-auto-refresh"
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
            data-testid="btn-refresh-processes"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {/* Overview Stat Cards */}
      {stats && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <Card data-testid="card-proc-total">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("processes.metricTotal")}
              </CardTitle>
              <Terminal className="w-4 h-4 text-muted-foreground" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold" data-testid="metric-total-processes">
                {stats.total_processes}
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {stats.running_processes} {t("processes.runningNow")}, {stats.sleeping_processes} {t("processes.sleeping")}
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-proc-cpu">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("processes.metricCpuTotal")}
              </CardTitle>
              <Cpu className="w-4 h-4 text-blue-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-blue-600 dark:text-blue-400" data-testid="metric-total-cpu">
                {stats.total_cpu_percent}%
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("processes.cgroupLimit2Cores")}
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-proc-ram">
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("processes.metricRamTotal")}
              </CardTitle>
              <Activity className="w-4 h-4 text-emerald-500" />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-emerald-600 dark:text-emerald-400" data-testid="metric-total-ram">
                {stats.total_mem_rss_mb} MB
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("processes.rssMemoryFootprint")}
              </p>
            </CardContent>
          </Card>

          <Card data-testid="card-proc-zombie" className={stats.zombie_processes > 0 ? "border-destructive/40 bg-destructive/5" : ""}>
            <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
              <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("processes.metricZombies")}
              </CardTitle>
              <AlertTriangle className={`w-4 h-4 ${stats.zombie_processes > 0 ? "text-destructive" : "text-muted-foreground"}`} />
            </CardHeader>
            <CardContent>
              <div className={`text-2xl font-bold ${stats.zombie_processes > 0 ? "text-destructive" : ""}`} data-testid="metric-total-zombies">
                {stats.zombie_processes}
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {stats.zombie_processes > 0 ? t("processes.defunctWarning") : t("processes.noZombiesDetected")}
              </p>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Process Table with search & sort */}
      <Card>
        <CardHeader className="p-4 border-b">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <CardTitle className="text-base font-semibold">
              {t("processes.activeProcessesTitle")}
            </CardTitle>

            <div className="flex items-center gap-2">
              {/* Sort by */}
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span>{t("common.sortBy")}:</span>
                <select
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value as any)}
                  className="h-8 text-xs rounded-md border border-input bg-background px-2 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                  data-testid="select-process-sort"
                >
                  <option value="cpu">% CPU (Highest)</option>
                  <option value="mem">% Memory RSS (Highest)</option>
                  <option value="pid">PID (Ascending)</option>
                </select>
              </div>

              {/* Search */}
              <div className="relative w-48 sm:w-60">
                <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder={t("processes.searchPlaceholder")}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="pl-8 text-xs h-8"
                  data-testid="input-search-processes"
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
                  <TableHead className="w-[100px]">{t("processes.thUser")}</TableHead>
                  <TableHead className="w-[120px]">{t("processes.thCpu")}</TableHead>
                  <TableHead className="w-[120px]">{t("processes.thMem")}</TableHead>
                  <TableHead className="w-[110px]">{t("processes.thStat")}</TableHead>
                  <TableHead className="w-[90px]">{t("processes.thTime")}</TableHead>
                  <TableHead>{t("processes.thCommand")}</TableHead>
                  <TableHead className="text-right w-[180px]">{t("processes.thActions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredAndSorted.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={8} className="h-32 text-center text-muted-foreground">
                      {t("processes.noProcessesFound")}
                    </TableCell>
                  </TableRow>
                ) : (
                  filteredAndSorted.map((proc) => (
                    <TableRow key={proc.pid} data-testid={`process-row-${proc.pid}`}>
                      {/* PID */}
                      <TableCell className="font-mono text-xs font-bold" data-testid={`proc-pid-${proc.pid}`}>
                        {proc.pid}
                      </TableCell>

                      {/* User */}
                      <TableCell className="text-xs text-muted-foreground font-mono">
                        {proc.user}
                      </TableCell>

                      {/* %CPU */}
                      <TableCell>
                        <div className="space-y-1">
                          <div className="text-xs font-semibold font-mono" data-testid={`proc-cpu-${proc.pid}`}>
                            {proc.cpu_percent}%
                          </div>
                          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
                            <div
                              className={`h-1.5 rounded-full ${
                                proc.cpu_percent > 50 ? "bg-red-500" : proc.cpu_percent > 20 ? "bg-amber-500" : "bg-blue-500"
                              }`}
                              style={{ width: `${Math.min(100, proc.cpu_percent)}%` }}
                            />
                          </div>
                        </div>
                      </TableCell>

                      {/* %MEM & RSS */}
                      <TableCell>
                        <div className="text-xs font-semibold font-mono" data-testid={`proc-mem-${proc.pid}`}>
                          {proc.mem_rss_mb} MB
                        </div>
                        <span className="text-[10px] text-muted-foreground font-mono">
                          {proc.mem_percent}%
                        </span>
                      </TableCell>

                      {/* STAT */}
                      <TableCell data-testid={`proc-stat-${proc.pid}`}>
                        {formatStatBadge(proc.stat)}
                      </TableCell>

                      {/* CPU Time */}
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {proc.cpu_time}
                      </TableCell>

                      {/* Command */}
                      <TableCell>
                        <div className="font-mono text-xs max-w-[340px] truncate text-foreground font-medium" data-testid={`proc-cmd-${proc.pid}`}>
                          {proc.command}
                        </div>
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        {proc.is_killable ? (
                          <div className="flex items-center justify-end gap-1.5">
                            {/* Graceful SIGTERM */}
                            <Button
                              size="sm"
                              variant="outline"
                              className="h-7 px-2 text-xs text-amber-600 hover:text-amber-700 hover:bg-amber-500/10 border-amber-300 dark:border-amber-800"
                              onClick={() => handleOpenKill(proc, "SIGTERM")}
                              data-testid={`btn-sigterm-${proc.pid}`}
                              title="Send SIGTERM (15)"
                            >
                              {t("processes.btnSigterm")}
                            </Button>

                            {/* Force SIGKILL */}
                            <Button
                              size="sm"
                              variant="destructive"
                              className="h-7 px-2 text-xs gap-1"
                              onClick={() => handleOpenKill(proc, "SIGKILL")}
                              data-testid={`btn-sigkill-${proc.pid}`}
                              title="Send SIGKILL (9)"
                            >
                              <XCircle className="w-3 h-3" />
                              {t("processes.btnSigkill")}
                            </Button>
                          </div>
                        ) : (
                          <span className="text-[11px] text-muted-foreground italic">
                            {t("processes.systemProtected")}
                          </span>
                        )}
                      </TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>

      {/* Kill Confirmation Modal */}
      <KillProcessDialog
        open={isKillOpen}
        onOpenChange={setIsKillOpen}
        process={selectedProcess}
        defaultSignal={selectedSignal}
        isSudoMode={false}
        onConfirm={handleExecuteKill}
      />
    </div>
  );
}
