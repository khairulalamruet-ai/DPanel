import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { useToast, Button, Card, CardContent } from "@builtpanel/ui";
import {
  ArrowRightLeft,
  Plus,
  RefreshCw,
  Server,
  CheckCircle2,
  AlertTriangle,
  HardDrive,
  Activity,
  AlertCircle,
} from "lucide-react";
import { TransferJob, RemoteMigrationPayload } from "./types";
import { TransfersTable } from "./TransfersTable";
import { NewTransferWizardDialog } from "./NewTransferWizardDialog";
import { TransferLogDialog } from "./TransferLogDialog";

export function MigrationPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [jobs, setJobs] = useState<TransferJob[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isError, setIsError] = useState(false);

  const [newTransferOpen, setNewTransferOpen] = useState(false);
  const [activeLogJob, setActiveLogJob] = useState<TransferJob | null>(null);

  const loadData = useCallback(async () => {
    try {
      setIsLoading(true);
      setIsError(false);
      const res = await apiFetch<TransferJob[]>("/api/v1/admin/migration/transfers");
      setJobs(res);

      // Keep active modal in sync if open
      setActiveLogJob((prev) => {
        if (!prev) return null;
        return res.find((j) => j.id === prev.id) || prev;
      });
    } catch {
      setIsError(true);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Handlers
  const handleStartTransfer = async (payload: RemoteMigrationPayload) => {
    const created = await apiFetch<TransferJob>("/api/v1/admin/migration/transfers", {
      method: "POST",
      body: JSON.stringify(payload),
    });
    toast({
      title: t("migration.startedTitle", "Migration Initiated"),
      description: t("migration.startedDesc", "Transfer task {{id}} for @{{domain}} has started.", {
        id: created.id,
        domain: payload.target_domain,
      }),
      variant: "success",
    });
    loadData();
    setActiveLogJob(created);
  };

  const handleRetryTransfer = async (id: string) => {
    await apiFetch(`/api/v1/admin/migration/transfers/${id}/retry`, { method: "POST" });
    toast({
      title: t("migration.retryingTitle", "Transfer Restarted"),
      variant: "success",
    });
    loadData();
  };

  const handleCancelTransfer = async (id: string) => {
    await apiFetch(`/api/v1/admin/migration/transfers/${id}/cancel`, { method: "POST" });
    toast({
      title: t("migration.cancelledTitle", "Transfer Aborted"),
      variant: "success",
    });
    loadData();
  };

  const handleDeleteTransfer = async (id: string) => {
    await apiFetch(`/api/v1/admin/migration/transfers/${id}`, { method: "DELETE" });
    toast({
      title: t("migration.deletedTitle", "Record Deleted"),
      variant: "success",
    });
    loadData();
  };

  // Metrics
  const activeCount = jobs.filter((j) => j.status === "running" || j.status === "pending").length;
  const completedCount = jobs.filter((j) => j.status === "completed").length;
  const totalCount = jobs.length;
  const successRate = totalCount > 0 ? Math.round((completedCount / totalCount) * 100) : 100;
  const totalSizeBytes = jobs.reduce((acc, j) => acc + (j.size_bytes || 0), 0);
  const totalSizeGb = Math.round((totalSizeBytes / (1024 * 1024 * 1024)) * 10) / 10;

  return (
    <div className="space-y-6" data-testid="migration-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="migration-page-title">
            {t("migration.pageTitle", "Server Migration & Account Import")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t(
              "migration.pageSubtitle",
              "Automate remote tenant migration from cPanel, CyberPanel, and OpenPanel directly into Built Panel."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            onClick={() => setNewTransferOpen(true)}
            className="gap-2 shrink-0"
            data-testid="btn-open-new-transfer"
          >
            <Plus className="h-4 w-4" />
            {t("migration.startMigrationBtn", "New Migration Transfer")}
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={loadData}
            disabled={isLoading}
            className="gap-2 shrink-0"
            data-testid="refresh-migration-btn"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {isError && (
        <div className="p-4 rounded-lg bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 border border-red-200 flex items-center gap-3">
          <AlertCircle className="h-5 w-5 shrink-0" />
          <div className="flex-1 text-sm">
            {t("migration.loadError", "Failed to load migration transfer status.")}
          </div>
          <Button variant="outline" size="sm" onClick={loadData}>
            {t("common.retry", "Retry")}
          </Button>
        </div>
      )}

      {/* Stats Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4" data-testid="migration-stats-grid">
        <Card className="border shadow-xs">
          <CardContent className="p-4 space-y-2">
            <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
              <span>{t("migration.activeTransfers", "Active Transfers")}</span>
              <Activity className="h-4 w-4 text-primary" />
            </div>
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold font-mono" data-testid="stat-active-transfers">
                {activeCount}
              </span>
              <span className="text-xs text-muted-foreground">in progress</span>
            </div>
          </CardContent>
        </Card>

        <Card className="border shadow-xs">
          <CardContent className="p-4 space-y-2">
            <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
              <span>{t("migration.completedMigrations", "Completed Accounts")}</span>
              <CheckCircle2 className="h-4 w-4 text-emerald-500" />
            </div>
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold font-mono" data-testid="stat-completed-transfers">
                {completedCount}
              </span>
              <span className="text-xs text-muted-foreground">restored</span>
            </div>
          </CardContent>
        </Card>

        <Card className="border shadow-xs">
          <CardContent className="p-4 space-y-2">
            <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
              <span>{t("migration.successRate", "Success Rate")}</span>
              <Server className="h-4 w-4 text-primary" />
            </div>
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold font-mono" data-testid="stat-success-rate">
                {successRate}%
              </span>
              <span className="text-xs text-muted-foreground">overall</span>
            </div>
          </CardContent>
        </Card>

        <Card className="border shadow-xs">
          <CardContent className="p-4 space-y-2">
            <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
              <span>{t("migration.totalStorage", "Migrated Data Volume")}</span>
              <HardDrive className="h-4 w-4 text-primary" />
            </div>
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold font-mono" data-testid="stat-migrated-storage">
                {totalSizeGb} GB
              </span>
              <span className="text-xs text-muted-foreground">transferred</span>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Transfers List */}
      <TransfersTable
        jobs={jobs}
        loading={isLoading}
        onOpenLogs={(job) => setActiveLogJob(job)}
        onRetry={handleRetryTransfer}
        onCancel={handleCancelTransfer}
        onDelete={handleDeleteTransfer}
      />

      {/* Dialogs */}
      <NewTransferWizardDialog
        open={newTransferOpen}
        onClose={() => setNewTransferOpen(false)}
        onSubmit={handleStartTransfer}
      />

      <TransferLogDialog
        job={activeLogJob}
        open={Boolean(activeLogJob)}
        onClose={() => setActiveLogJob(null)}
        onRetry={handleRetryTransfer}
        onCancel={handleCancelTransfer}
      />
    </div>
  );
}
