import React, { useState } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Badge,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Search,
  Terminal,
  RotateCw,
  StopCircle,
  Trash2,
  AlertTriangle,
  Server,
  Globe,
  HardDrive,
} from "lucide-react";
import { TransferJob } from "./types";

interface TransfersTableProps {
  jobs: TransferJob[];
  loading: boolean;
  onOpenLogs: (job: TransferJob) => void;
  onRetry: (id: string) => Promise<void>;
  onCancel: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}

export function TransfersTable({
  jobs,
  loading,
  onOpenLogs,
  onRetry,
  onCancel,
  onDelete,
}: TransfersTableProps) {
  const { t } = useTranslation();

  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [deleteTarget, setDeleteTarget] = useState<TransferJob | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const filtered = jobs.filter((j) => {
    if (statusFilter !== "all" && j.status !== statusFilter) return false;
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return (
      j.id.toLowerCase().includes(q) ||
      j.tenant_username.toLowerCase().includes(q) ||
      j.target_domain.toLowerCase().includes(q) ||
      (j.source_host && j.source_host.toLowerCase().includes(q))
    );
  });

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await onDelete(deleteTarget.id);
      setDeleteTarget(null);
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="transfers-table-section">
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder={t("migration.searchTransfers", "Search transfers, tenants...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8 text-xs"
            data-testid="input-search-transfers"
          />
        </div>

        {/* Filter Pills */}
        <div className="flex items-center gap-1.5 overflow-x-auto text-xs">
          {["all", "running", "completed", "failed"].map((status) => (
            <button
              key={status}
              type="button"
              onClick={() => setStatusFilter(status)}
              className={`px-2.5 py-1 rounded-md capitalize transition-colors ${
                statusFilter === status
                  ? "bg-primary text-primary-foreground font-semibold"
                  : "bg-muted text-muted-foreground hover:bg-muted/80"
              }`}
              data-testid={`filter-${status}`}
            >
              {status}
            </button>
          ))}
        </div>
      </div>

      <div className="rounded-lg border bg-card overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("migration.transferIdCol", "Transfer ID")}</TableHead>
              <TableHead>{t("migration.sourceCol", "Source Platform")}</TableHead>
              <TableHead>{t("migration.targetCol", "Destination")}</TableHead>
              <TableHead>{t("migration.stepCol", "Current Stage & Progress")}</TableHead>
              <TableHead>{t("common.status", "Status")}</TableHead>
              <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("common.loading", "Loading transfer jobs...")}
                </TableCell>
              </TableRow>
            ) : filtered.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("migration.noTransfers", "No migration transfers found.")}
                </TableCell>
              </TableRow>
            ) : (
              filtered.map((job) => (
                <TableRow
                  key={job.id}
                  data-testid={`transfer-row-${job.id}`}
                  className="cursor-pointer hover:bg-muted/40 transition-colors"
                  onClick={() => onOpenLogs(job)}
                >
                  <TableCell className="font-mono text-xs font-bold">
                    <span className="text-primary hover:underline">{job.id}</span>
                  </TableCell>

                  <TableCell>
                    <div className="space-y-0.5">
                      <Badge variant="outline" className="font-mono text-[10px] uppercase">
                        {job.source_type}
                      </Badge>
                      <span className="text-xs text-muted-foreground block font-mono truncate max-w-[140px]">
                        {job.source_host || "Local File"}
                      </span>
                    </div>
                  </TableCell>

                  <TableCell>
                    <div className="space-y-0.5">
                      <span className="font-bold text-xs block font-mono">{job.tenant_username}</span>
                      <span className="text-[11px] text-muted-foreground flex items-center gap-1">
                        <Globe className="h-3 w-3" />
                        {job.target_domain}
                      </span>
                    </div>
                  </TableCell>

                  <TableCell className="w-56">
                    <div className="space-y-1">
                      <div className="flex items-center justify-between text-[11px] font-mono">
                        <span className="truncate max-w-[150px]">{job.current_step}</span>
                        <span className="font-bold">{job.progress_percent}%</span>
                      </div>
                      <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
                        <div
                          className={`h-full rounded-full transition-all duration-300 ${
                            job.status === "failed"
                              ? "bg-destructive"
                              : job.status === "completed"
                              ? "bg-emerald-500"
                              : "bg-primary"
                          }`}
                          style={{ width: `${job.progress_percent}%` }}
                        />
                      </div>
                    </div>
                  </TableCell>

                  <TableCell>
                    <Badge
                      variant={
                        job.status === "completed"
                          ? "success"
                          : job.status === "running"
                          ? "default"
                          : job.status === "failed"
                          ? "destructive"
                          : "secondary"
                      }
                      data-testid={`transfer-badge-${job.id}`}
                    >
                      {job.status.toUpperCase()}
                    </Badge>
                  </TableCell>

                  <TableCell className="text-right" onClick={(e) => e.stopPropagation()}>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => onOpenLogs(job)}
                        data-testid={`btn-view-logs-${job.id}`}
                        className="h-8 text-xs gap-1"
                        title="View Log & Steps"
                      >
                        <Terminal className="h-3.5 w-3.5" />
                        <span>{t("migration.viewLogs", "Logs")}</span>
                      </Button>

                      {job.status === "running" && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => onCancel(job.id)}
                          data-testid={`btn-cancel-row-${job.id}`}
                          className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                          title="Abort Transfer"
                        >
                          <StopCircle className="h-3.5 w-3.5" />
                        </Button>
                      )}

                      {job.status === "failed" && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => onRetry(job.id)}
                          data-testid={`btn-retry-row-${job.id}`}
                          className="h-8 w-8 p-0 text-primary hover:bg-primary/10"
                          title="Retry Migration"
                        >
                          <RotateCw className="h-3.5 w-3.5" />
                        </Button>
                      )}

                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setDeleteTarget(job)}
                        data-testid={`btn-delete-transfer-${job.id}`}
                        className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                        title="Delete Record"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      {/* Delete Confirmation Dialog */}
      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <div className="flex items-center gap-2 text-destructive">
                <AlertTriangle className="h-5 w-5" />
                <DialogTitle>{t("migration.deleteRecordTitle", "Delete Transfer Record")}</DialogTitle>
              </div>
              <DialogDescription className="text-xs">
                {t(
                  "migration.deleteRecordDesc",
                  "Remove migration record for {{tenant}} ({{id}})? Target restored services will not be affected.",
                  { tenant: deleteTarget.tenant_username, id: deleteTarget.id }
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={isDeleting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                variant="destructive"
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                data-testid="btn-confirm-delete-transfer"
              >
                {isDeleting ? t("common.deleting", "Deleting...") : t("common.delete", "Delete")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
