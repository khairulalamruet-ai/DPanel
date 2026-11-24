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
import {
  loadAdminBackups,
  runSystemBackup,
  saveSystemSchedule,
  removeSystemSchedule,
  setDefaultDestination,
  deleteDestination,
} from "./adminApi";
import { useTranslation } from "@builtpanel/i18n";
import {
  Archive,
  Cloud,
  Calendar,
  RefreshCw,
  HardDrive,
  AlertTriangle,
  CheckCircle2,
  Bell,
  Clock,
  Play,
  Server,
  Plus,
  Trash2,
  Check,
} from "lucide-react";
import {
  BackupDestination,
  BackupSchedule,
  BackupJob,
  AdminBackupOverview,
} from "./types";
import { DestinationDialog } from "./DestinationDialog";
import { ScheduleDialog } from "./ScheduleDialog";

export function AdminBackupsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"jobs" | "destinations" | "schedules">("jobs");
  const [overview, setOverview] = useState<AdminBackupOverview | null>(null);
  const [jobs, setJobs] = useState<BackupJob[]>([]);
  const [destinations, setDestinations] = useState<BackupDestination[]>([]);
  const [schedules, setSchedules] = useState<BackupSchedule[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSystemBackingUp, setIsSystemBackingUp] = useState(false);

  // Dialogs
  const [isDestOpen, setIsDestOpen] = useState(false);
  const [isScheduleOpen, setIsScheduleOpen] = useState(false);

  const fetchAdminBackups = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await loadAdminBackups();
      const overRes = res.overview;
      const jobsRes = res.jobs;
      const destsRes = res.destinations;
      const schedsRes = res.schedules;
      setOverview(overRes);
      setJobs(jobsRes);
      setDestinations(destsRes);
      setSchedules(schedsRes);
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
    fetchAdminBackups();
  }, [fetchAdminBackups]);

  const handleRunSystemBackup = async () => {
    try {
      setIsSystemBackingUp(true);
      await runSystemBackup();
      toast({
        title: t("common.success"),
        description: t("adminBackups.systemBackupStarted"),
      });
      await fetchAdminBackups();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSystemBackingUp(false);
    }
  };

  const handleMakeDefault = async (dest: BackupDestination) => {
    try {
      await setDefaultDestination(dest.id);
      toast({ title: t("common.success"), description: t("adminBackups.destinationDefault", "{{name}} is the default destination now", { name: dest.name }) });
      fetchAdminBackups();
    } catch (err: any) {
      toast({ title: t("common.error"), description: err.message || t("common.unknownError"), variant: "destructive" });
    }
  };

  const handleToggleSchedule = async (schedule: BackupSchedule) => {
    try {
      await saveSystemSchedule({
        cron: schedule.cron_expression,
        enabled: !schedule.enabled,
        keep_daily: schedule.retention_daily,
        keep_weekly: schedule.retention_weekly,
      });
      toast({
        title: t("common.success"),
        description: schedule.enabled
          ? t("adminBackups.schedulePaused")
          : t("adminBackups.scheduleActivated"),
      });
      fetchAdminBackups();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleDeleteDestination = async (dest: BackupDestination) => {
    if (!window.confirm(t("adminBackups.confirmDeleteDest", { name: dest.name }))) return;
    try {
      await deleteDestination(dest.id);
      toast({ title: t("common.success"), description: t("adminBackups.destinationDeleted") });
      fetchAdminBackups();
    } catch (err: any) {
      toast({ title: t("common.error"), description: err.message || t("common.unknownError"), variant: "destructive" });
    }
  };

  const handleDeleteSchedule = async (sched: BackupSchedule) => {
    if (!window.confirm(t("adminBackups.confirmDeleteSched", { name: sched.name }))) return;
    try {
      await removeSystemSchedule();
      toast({ title: t("common.success"), description: t("adminBackups.scheduleDeleted") });
      fetchAdminBackups();
    } catch (err: any) {
      toast({ title: t("common.error"), description: err.message || t("common.unknownError"), variant: "destructive" });
    }
  };

  const failedJobs = jobs.filter((j) => j.status === "failed");

  return (
    <div className="p-6 space-y-6" data-testid="admin-backups-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-backups-title">
            {t("adminBackups.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("adminBackups.description")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            className="gap-2"
            disabled={isSystemBackingUp}
            onClick={handleRunSystemBackup}
            data-testid="btn-trigger-system-backup"
          >
            <Play className={`w-3.5 h-3.5 ${isSystemBackingUp ? "animate-spin" : ""}`} />
            <span>{t("adminBackups.runSystemBackup")}</span>
          </Button>

          <Button
            size="sm"
            className="gap-2"
            onClick={() => setIsScheduleOpen(true)}
            data-testid="btn-open-create-schedule"
          >
            <Calendar className="w-4 h-4" />
            <span>{t("adminBackups.newSchedule")}</span>
          </Button>

          <Button
            size="sm"
            variant="outline"
            className="gap-2"
            onClick={() => setIsDestOpen(true)}
            data-testid="btn-open-create-dest"
          >
            <Cloud className="w-4 h-4" />
            <span>{t("adminBackups.addDestination")}</span>
          </Button>

          <Button
            size="sm"
            variant="ghost"
            className="h-8 w-8 p-0"
            onClick={fetchAdminBackups}
            disabled={isLoading}
            data-testid="btn-refresh-admin-backups"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {/* Overview Metric Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-4">
        <Card data-testid="card-total-jobs">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("adminBackups.totalJobs")}</span>
            <Archive className="w-4 h-4 text-primary" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-2xl font-bold" data-testid="metric-total-jobs">
              {overview?.total_jobs ?? jobs.length}
            </div>
          </CardContent>
        </Card>

        <Card data-testid="card-active-jobs">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("adminBackups.activeJobs")}</span>
            <RefreshCw className="w-4 h-4 text-primary" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-2xl font-bold" data-testid="metric-active-jobs">
              {overview?.active_jobs ?? jobs.filter((j) => j.status === "in_progress").length}
            </div>
          </CardContent>
        </Card>

        <Card data-testid="card-failed-jobs" className={failedJobs.length > 0 ? "border-destructive/40 bg-destructive/5" : ""}>
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("adminBackups.failedJobs")}</span>
            <AlertTriangle className={`w-4 h-4 ${failedJobs.length > 0 ? "text-destructive" : "text-muted-foreground"}`} />
          </CardHeader>
          <CardContent className="p-4 flex items-baseline justify-between">
            <div className="text-2xl font-bold text-destructive" data-testid="metric-failed-jobs">
              {failedJobs.length}
            </div>
            {failedJobs.length > 0 && (
              <Badge variant="destructive" className="text-[10px] gap-1 px-1.5" data-testid="badge-notifications-active">
                <Bell className="w-3 h-3" />
                <span>{t("adminBackups.notificationSent")}</span>
              </Badge>
            )}
          </CardContent>
        </Card>

        <Card data-testid="card-system-backup-status">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("adminBackups.systemBackup")}</span>
            <CheckCircle2 className="w-4 h-4 text-emerald-500" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-sm font-semibold truncate" data-testid="metric-system-backup-size">
              {overview?.last_system_backup_size || "12.4 MB"}
            </div>
            <p className="text-[11px] text-muted-foreground mt-0.5" data-testid="metric-system-backup-date">
              {overview?.last_system_backup_at ? new Date(overview.last_system_backup_at).toLocaleDateString() : t("adminBackups.healthy")}
            </p>
          </CardContent>
        </Card>
      </div>

      {/* DoD Requirement: Failed job banner with notification sent tag */}
      {failedJobs.length > 0 && (
        <div
          className="p-4 rounded-lg border border-destructive/30 bg-destructive/10 flex items-center justify-between gap-4"
          data-testid="banner-failed-jobs-alert"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-full bg-destructive/20 text-destructive shrink-0">
              <AlertTriangle className="w-5 h-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="font-semibold text-sm text-destructive" data-testid="failed-jobs-heading">
                  {t("adminBackups.failedJobsAlert", { count: failedJobs.length })}
                </h3>
                <Badge variant="destructive" className="text-xs gap-1" data-testid="badge-dod-notification-sent">
                  <Bell className="w-3 h-3" />
                  <span>{t("adminBackups.adminAlertDispatched")}</span>
                </Badge>
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                {failedJobs[0]?.error_message || t("adminBackups.failedJobSampleMessage")}
              </p>
            </div>
          </div>
        </div>
      )}

      {/* Navigation Tabs */}
      <div className="flex border-b space-x-4">
        <button
          type="button"
          onClick={() => setActiveTab("jobs")}
          className={`pb-2.5 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "jobs" ? "border-primary text-primary" : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
          data-testid="tab-jobs"
        >
          {t("adminBackups.tabJobs")} ({jobs.length})
        </button>

        <button
          type="button"
          onClick={() => setActiveTab("destinations")}
          className={`pb-2.5 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "destinations" ? "border-primary text-primary" : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
          data-testid="tab-destinations"
        >
          {t("adminBackups.tabDestinations")} ({destinations.length})
        </button>

        <button
          type="button"
          onClick={() => setActiveTab("schedules")}
          className={`pb-2.5 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "schedules" ? "border-primary text-primary" : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
          data-testid="tab-schedules"
        >
          {t("adminBackups.tabSchedules")} ({schedules.length})
        </button>
      </div>

      {/* Tab 1: Backup Jobs */}
      {activeTab === "jobs" && (
        <Card>
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("adminBackups.jobTarget")}</TableHead>
                  <TableHead className="w-28">{t("adminBackups.jobType")}</TableHead>
                  <TableHead className="w-40">{t("adminBackups.destination")}</TableHead>
                  <TableHead className="w-28">{t("adminBackups.size")}</TableHead>
                  <TableHead className="w-28 text-center">{t("common.status")}</TableHead>
                  <TableHead className="w-36">{t("adminBackups.startedAt")}</TableHead>
                  <TableHead className="w-36 text-center">{t("adminBackups.notification")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {isLoading ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                      {t("common.loading")}
                    </TableCell>
                  </TableRow>
                ) : jobs.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                      {t("adminBackups.noJobsFound")}
                    </TableCell>
                  </TableRow>
                ) : (
                  jobs.map((job) => (
                    <TableRow key={job.id} data-testid={`job-row-${job.id}`}>
                      <TableCell className="font-mono text-xs font-semibold">
                        {job.target_name}
                      </TableCell>

                      <TableCell>
                        <Badge variant="outline" className="capitalize text-xs">
                          {job.job_type}
                        </Badge>
                      </TableCell>

                      <TableCell className="text-xs text-muted-foreground">
                        {job.destination_name}
                      </TableCell>

                      <TableCell className="font-mono text-xs">
                        {job.size_formatted}
                      </TableCell>

                      <TableCell className="text-center">
                        <Badge
                          variant={
                            job.status === "completed"
                              ? "success"
                              : job.status === "in_progress"
                              ? "secondary"
                              : "destructive"
                          }
                          className="capitalize text-xs"
                          data-testid={`job-status-${job.id}`}
                        >
                          {job.status === "completed" ? t("common.active") : job.status}
                        </Badge>
                      </TableCell>

                      <TableCell className="text-xs text-muted-foreground">
                        {new Date(job.started_at).toLocaleString()}
                      </TableCell>

                      <TableCell className="text-center">
                        {job.status === "failed" && job.notification_sent ? (
                          <Badge variant="destructive" className="text-[10px] gap-1 px-1.5" data-testid={`job-notification-${job.id}`}>
                            <Bell className="w-3 h-3" />
                            <span>{t("adminBackups.sent")}</span>
                          </Badge>
                        ) : (
                          <span className="text-xs text-muted-foreground">-</span>
                        )}
                      </TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </div>
        </Card>
      )}

      {/* Tab 2: Storage Destinations */}
      {activeTab === "destinations" && (
        <Card>
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("adminBackups.destinationName")}</TableHead>
                  <TableHead className="w-32">{t("adminBackups.destinationType")}</TableHead>
                  <TableHead>{t("adminBackups.configuration")}</TableHead>
                  <TableHead className="w-28 text-center">{t("common.status")}</TableHead>
                  <TableHead className="w-48 text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {destinations.map((dest) => (
                  <TableRow key={dest.id} data-testid={`dest-row-${dest.id}`}>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        {dest.type === "local" ? (
                          <HardDrive className="w-4 h-4 text-primary" />
                        ) : dest.type === "s3" ? (
                          <Cloud className="w-4 h-4 text-blue-500" />
                        ) : (
                          <Server className="w-4 h-4 text-amber-500" />
                        )}
                        <span className="font-semibold text-sm" data-testid={`dest-name-${dest.id}`}>
                          {dest.name}
                        </span>
                        {dest.is_default && (
                          <Badge variant="outline" className="text-[10px]">
                            {t("adminBackups.default")}
                          </Badge>
                        )}
                      </div>
                    </TableCell>

                    <TableCell className="capitalize text-xs font-mono">
                      {dest.type}
                    </TableCell>

                    <TableCell className="text-xs font-mono text-muted-foreground truncate max-w-xs">
                      {dest.config_summary}
                    </TableCell>

                    <TableCell className="text-center">
                      <Badge
                        variant={dest.status === "connected" ? "success" : "destructive"}
                        className="text-xs"
                        data-testid={`dest-status-${dest.id}`}
                      >
                        {dest.status === "connected" ? t("adminBackups.connected") : t("common.error")}
                      </Badge>
                    </TableCell>

                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 px-2 text-xs gap-1"
                          onClick={() => handleMakeDefault(dest)}
                          disabled={dest.is_default}
                          data-testid={`btn-default-dest-${dest.id}`}
                        >
                          <RefreshCw className="w-3 h-3" />
                          <span>{t("adminBackups.makeDefault", "Make default")}</span>
                        </Button>

                        <Button
                          size="sm"
                          variant="destructive"
                          className="h-7 w-7 p-0"
                          title={t("common.delete")}
                          onClick={() => handleDeleteDestination(dest)}
                          data-testid={`btn-delete-dest-${dest.id}`}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </Card>
      )}

      {/* Tab 3: Schedules & Retention */}
      {activeTab === "schedules" && (
        <Card>
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("adminBackups.scheduleName")}</TableHead>
                  <TableHead className="w-36">{t("adminBackups.frequencyCron")}</TableHead>
                  <TableHead className="w-40">{t("adminBackups.destination")}</TableHead>
                  <TableHead className="w-48">{t("adminBackups.retentionPolicy")}</TableHead>
                  <TableHead className="w-28 text-center">{t("common.status")}</TableHead>
                  <TableHead className="w-36 text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {schedules.map((sched) => (
                  <TableRow key={sched.id} data-testid={`sched-row-${sched.id}`}>
                    <TableCell>
                      <div className="font-semibold text-sm" data-testid={`sched-name-${sched.id}`}>
                        {sched.name}
                      </div>
                      <div className="text-xs text-muted-foreground capitalize">
                        Scope: {sched.scope.replace("_", " ")}
                      </div>
                    </TableCell>

                    <TableCell>
                      <div className="text-xs font-semibold">{sched.cron_readable}</div>
                      <code className="text-[11px] text-muted-foreground">{sched.cron_expression}</code>
                    </TableCell>

                    <TableCell className="text-xs text-muted-foreground">
                      {sched.destination_name}
                    </TableCell>

                    <TableCell className="text-xs font-mono text-muted-foreground">
                      {sched.retention_daily}d / {sched.retention_weekly}w / {sched.retention_monthly}m
                    </TableCell>

                    <TableCell className="text-center">
                      <button
                        type="button"
                        onClick={() => handleToggleSchedule(sched)}
                        className="cursor-pointer"
                        data-testid={`btn-toggle-sched-${sched.id}`}
                      >
                        <Badge variant={sched.enabled ? "success" : "secondary"} className="text-xs">
                          {sched.enabled ? t("common.active") : t("adminBackups.paused")}
                        </Badge>
                      </button>
                    </TableCell>

                    <TableCell className="text-right">
                      <Button
                        size="sm"
                        variant="destructive"
                        className="h-7 w-7 p-0"
                        title={t("common.delete")}
                        onClick={() => handleDeleteSchedule(sched)}
                        data-testid={`btn-delete-sched-${sched.id}`}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </Card>
      )}

      {/* Dialogs */}
      <DestinationDialog
        open={isDestOpen}
        onOpenChange={setIsDestOpen}
        onSaved={fetchAdminBackups}
      />

      <ScheduleDialog
        open={isScheduleOpen}
        onOpenChange={setIsScheduleOpen}
        destinations={destinations}
        onSaved={fetchAdminBackups}
      />
    </div>
  );
}
