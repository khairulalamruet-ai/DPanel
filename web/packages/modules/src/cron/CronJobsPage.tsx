import React, { useState, useEffect, useCallback } from "react";
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
import { useTranslation } from "@builtpanel/i18n";
import {
  Clock,
  Play,
  Terminal,
  Plus,
  RefreshCw,
  Search,
  CheckCircle2,
  AlertTriangle,
  Trash2,
  Power,
  Calendar,
  Layers,
} from "lucide-react";
import { CronJob } from "./types";
import { listCronJobs, setCronEnabled, runCronJob, deleteCronJob } from "./api";
import { CreateCronJobDialog } from "./CreateCronJobDialog";
import { CronOutputModal } from "./CronOutputModal";

export function CronJobsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [runningJobId, setRunningJobId] = useState<string | null>(null);

  // Dialogs
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [selectedJobForOutput, setSelectedJobForOutput] = useState<CronJob | null>(null);
  const [isOutputOpen, setIsOutputOpen] = useState(false);

  const fetchJobs = useCallback(async () => {
    try {
      setIsLoading(true);
      const { jobs: res } = await listCronJobs();
      setJobs(res);
      // update selected job if open
      if (selectedJobForOutput) {
        const updated = res.find((j) => j.id === selectedJobForOutput.id);
        if (updated) setSelectedJobForOutput(updated);
      }
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast, selectedJobForOutput]);

  useEffect(() => {
    fetchJobs();
  }, [fetchJobs]);

  const handleToggleJob = async (job: CronJob) => {
    try {
      const updated = await setCronEnabled(job, !job.enabled);
      toast({
        title: t("common.success"),
        description: updated.enabled
          ? t("cron.jobEnabledSuccess")
          : t("cron.jobDisabledSuccess"),
      });
      fetchJobs();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleRunNow = async (job: CronJob) => {
    try {
      setRunningJobId(job.id);
      await runCronJob(job.id);

      toast({
        title: t("common.success"),
        description: t("cron.jobRunQueued"),
      });

      // The result shows as the job's last run within about a minute.
      setTimeout(() => void fetchJobs(), 5000);
      setTimeout(() => void fetchJobs(), 30000);
      await fetchJobs();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setRunningJobId(null);
    }
  };

  const handleDeleteJob = async (job: CronJob) => {
    if (!window.confirm(t("cron.confirmDelete", { name: job.name }))) return;

    try {
      await deleteCronJob(job.id);
      toast({
        title: t("common.success"),
        description: t("cron.jobDeletedSuccess"),
      });
      fetchJobs();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleOpenOutput = (job: CronJob) => {
    setSelectedJobForOutput(job);
    setIsOutputOpen(true);
  };

  // Calculations for metric cards
  const totalJobs = jobs.length;
  const activeJobs = jobs.filter((j) => j.enabled).length;
  const failedJobs = jobs.filter((j) => j.last_status === "failed" || (j.last_exit_code != null && j.last_exit_code > 0)).length;
  const nextScheduled = jobs
    .filter((j) => j.enabled && j.next_run_at)
    .sort((a, b) => new Date(a.next_run_at as string).getTime() - new Date(b.next_run_at as string).getTime())[0];

  const filteredJobs = jobs.filter((j) => {
    if (!searchQuery.trim()) return true;
    const q = searchQuery.toLowerCase();
    return (
      j.name.toLowerCase().includes(q) ||
      j.command.toLowerCase().includes(q) ||
      j.schedule.includes(q)
    );
  });

  return (
    <div className="p-6 space-y-6" data-testid="cron-jobs-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="cron-page-title">
            {t("cron.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("cron.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={fetchJobs}
            disabled={isLoading}
            data-testid="btn-refresh-cron"
          >
            <RefreshCw className={`w-3.5 h-3.5 mr-1.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>

          <Button
            size="sm"
            onClick={() => setIsCreateOpen(true)}
            data-testid="btn-open-create-cron"
            className="gap-1.5"
          >
            <Plus className="w-4 h-4" />
            {t("cron.createButton")}
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <Card data-testid="card-total-cron">
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("cron.metricTotal")}
            </CardTitle>
            <Clock className="w-4 h-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold" data-testid="metric-total-cron">
              {totalJobs}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("cron.metricTotalSubtitle")}
            </p>
          </CardContent>
        </Card>

        <Card data-testid="card-active-cron">
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("cron.metricActive")}
            </CardTitle>
            <CheckCircle2 className="w-4 h-4 text-green-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-green-600 dark:text-green-400" data-testid="metric-active-cron">
              {activeJobs}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("cron.metricActiveSubtitle")}
            </p>
          </CardContent>
        </Card>

        <Card data-testid="card-failed-cron" className={failedJobs > 0 ? "border-destructive/40 bg-destructive/5" : ""}>
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("cron.metricFailed")}
            </CardTitle>
            <AlertTriangle className={`w-4 h-4 ${failedJobs > 0 ? "text-destructive" : "text-muted-foreground"}`} />
          </CardHeader>
          <CardContent>
            <div className={`text-2xl font-bold ${failedJobs > 0 ? "text-destructive" : ""}`} data-testid="metric-failed-cron">
              {failedJobs}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("cron.metricFailedSubtitle")}
            </p>
          </CardContent>
        </Card>

        <Card data-testid="card-next-cron">
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("cron.metricNextRun")}
            </CardTitle>
            <Calendar className="w-4 h-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-sm font-semibold truncate" data-testid="metric-next-cron-time">
              {nextScheduled ? new Date(nextScheduled.next_run_at as string).toLocaleTimeString() : "—"}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5 truncate" data-testid="metric-next-cron-name">
              {nextScheduled ? nextScheduled.name : t("cron.noneScheduled")}
            </p>
          </CardContent>
        </Card>
      </div>

      {/* Main Content Area */}
      <Card>
        <CardHeader className="p-4 border-b">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <CardTitle className="text-base font-semibold">
              {t("cron.jobsListTitle")}
            </CardTitle>
            <div className="relative w-full sm:w-64">
              <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
              <Input
                placeholder={t("cron.searchPlaceholder")}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-8 text-xs h-8"
                data-testid="input-search-cron"
              />
            </div>
          </div>
        </CardHeader>

        <CardContent className="p-0">
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[40px]"></TableHead>
                  <TableHead>{t("cron.thJob")}</TableHead>
                  <TableHead>{t("cron.thSchedule")}</TableHead>
                  <TableHead>{t("cron.thNextRun")}</TableHead>
                  <TableHead>{t("cron.thLastRun")}</TableHead>
                  <TableHead>{t("cron.thStatus")}</TableHead>
                  <TableHead>{t("cron.thState")}</TableHead>
                  <TableHead className="text-right">{t("cron.thActions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredJobs.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={8} className="h-36 text-center text-muted-foreground">
                      <Clock className="w-8 h-8 mx-auto mb-2 opacity-30" />
                      <p>{t("cron.noJobsFound")}</p>
                    </TableCell>
                  </TableRow>
                ) : (
                  filteredJobs.map((job) => {
                    const isRunning = runningJobId === job.id || job.last_status === "running";
                    const isSuccess = job.last_status === "success" || job.last_exit_code === 0;
                    const isFailed = job.last_status === "failed" || (job.last_exit_code != null && job.last_exit_code > 0);

                    return (
                      <TableRow key={job.id} data-testid={`cron-row-${job.id}`}>
                        <TableCell>
                          {isSuccess ? (
                            <CheckCircle2 className="w-4 h-4 text-green-500" />
                          ) : isFailed ? (
                            <AlertTriangle className="w-4 h-4 text-destructive" />
                          ) : (
                            <Clock className="w-4 h-4 text-muted-foreground" />
                          )}
                        </TableCell>

                        {/* Name & Command */}
                        <TableCell>
                          <div className="space-y-0.5">
                            <span className="font-semibold text-sm" data-testid={`cron-name-${job.id}`}>
                              {job.name}
                            </span>
                            {job.description && (
                              <p className="text-xs text-muted-foreground line-clamp-1">
                                {job.description}
                              </p>
                            )}
                            <div className="inline-flex items-center gap-1 font-mono text-[11px] bg-muted px-1.5 py-0.5 rounded text-foreground/80 max-w-[320px] truncate">
                              $ {job.command}
                            </div>
                          </div>
                        </TableCell>

                        {/* Schedule */}
                        <TableCell>
                          <div className="space-y-1">
                            <Badge variant="outline" className="font-mono text-xs" data-testid={`cron-schedule-${job.id}`}>
                              {job.schedule}
                            </Badge>
                            {job.schedule_human && (
                              <p className="text-[11px] text-muted-foreground">
                                {job.schedule_human}
                              </p>
                            )}
                          </div>
                        </TableCell>

                        {/* Next Run */}
                        <TableCell className="text-xs text-muted-foreground">
                          {job.enabled && job.next_run_at ? (
                            new Date(job.next_run_at).toLocaleString()
                          ) : (
                            <span className="italic">{t("cron.paused")}</span>
                          )}
                        </TableCell>

                        {/* Last Run */}
                        <TableCell className="text-xs text-muted-foreground">
                          {job.last_run_at ? (
                            <div>
                              <div>{new Date(job.last_run_at).toLocaleString()}</div>
                              {job.last_duration_ms != null && (
                                <span className="text-[11px] opacity-75">
                                  ({job.last_duration_ms} ms)
                                </span>
                              )}
                            </div>
                          ) : (
                            "—"
                          )}
                        </TableCell>

                        {/* Status Badge */}
                        <TableCell>
                          {isRunning ? (
                            <Badge variant="outline" className="gap-1 animate-pulse" data-testid={`cron-status-${job.id}`}>
                              <RefreshCw className="w-3 h-3 animate-spin" />
                              {t("cron.running")}
                            </Badge>
                          ) : isSuccess ? (
                            <Badge variant="outline" className="text-green-600 border-green-300 dark:border-green-800 bg-green-500/10 gap-1" data-testid={`cron-status-${job.id}`}>
                              <CheckCircle2 className="w-3 h-3" />
                              {t("cron.statusSuccess")}
                            </Badge>
                          ) : isFailed ? (
                            <Badge variant="destructive" className="gap-1" data-testid={`cron-status-${job.id}`}>
                              <AlertTriangle className="w-3 h-3" />
                              {t("cron.statusFailed")}
                            </Badge>
                          ) : (
                            <Badge variant="outline" className="text-muted-foreground" data-testid={`cron-status-${job.id}`}>
                              {t("cron.statusIdle")}
                            </Badge>
                          )}
                        </TableCell>

                        {/* Enable/Disable Toggle */}
                        <TableCell>
                          <Button
                            size="sm"
                            variant={job.enabled ? "secondary" : "outline"}
                            className={`h-7 px-2 text-xs gap-1 ${job.enabled ? "text-green-600 dark:text-green-400 font-medium" : "text-muted-foreground"}`}
                            onClick={() => handleToggleJob(job)}
                            data-testid={`btn-toggle-cron-${job.id}`}
                          >
                            <Power className="w-3 h-3" />
                            {job.enabled ? t("cron.enabled") : t("cron.disabled")}
                          </Button>
                        </TableCell>

                        {/* Actions */}
                        <TableCell className="text-right">
                          <div className="flex items-center justify-end gap-1.5">
                            <Button
                              size="sm"
                              variant="outline"
                              className="h-7 px-2 text-xs gap-1"
                              disabled={isRunning}
                              onClick={() => handleRunNow(job)}
                              data-testid={`btn-run-cron-${job.id}`}
                              title={t("cron.runNow")}
                            >
                              <Play className={`w-3 h-3 ${isRunning ? "animate-spin" : ""}`} />
                              <span className="hidden md:inline">{t("cron.runNow")}</span>
                            </Button>

                            <Button
                              size="sm"
                              variant="ghost"
                              className="h-7 px-2 text-xs gap-1 text-muted-foreground hover:text-foreground"
                              onClick={() => handleOpenOutput(job)}
                              data-testid={`btn-view-output-${job.id}`}
                              title={t("cron.viewOutput")}
                            >
                              <Terminal className="w-3 h-3" />
                              <span className="hidden md:inline">{t("cron.viewOutput")}</span>
                            </Button>

                            <Button
                              size="sm"
                              variant="ghost"
                              className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive"
                              onClick={() => handleDeleteJob(job)}
                              data-testid={`btn-delete-cron-${job.id}`}
                              title={t("common.delete")}
                            >
                              <Trash2 className="w-3.5 h-3.5" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  })
                )}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>

      {/* Dialogs */}
      <CreateCronJobDialog
        open={isCreateOpen}
        onOpenChange={setIsCreateOpen}
        onSaved={fetchJobs}
      />

      <CronOutputModal
        job={selectedJobForOutput}
        open={isOutputOpen}
        onOpenChange={setIsOutputOpen}
        onRunNow={handleRunNow}
      />
    </div>
  );
}
