import React, { useState, useEffect } from "react";
import {
  Button,
  Input,
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import { listOperations, retryOperation } from "./api";
import { useSession } from "@builtpanel/auth";
import {
  Activity,
  Filter,
  RotateCcw,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Clock,
  Code2,
  Layers,
} from "lucide-react";
import type { OperationItem } from "./types";
import { OperationDetailDialog } from "./OperationDetailDialog";
import { ApplyToAllDialog } from "./ApplyToAllDialog";

/** Rolling an operation over all nodes has no backend yet (phase 16), so its button is off. */
const CLUSTER_ROLLOUT = false;

export function OperationsPage() {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();
  const { toast } = useToast();
  const { hasPermission, isLoading: isSessionLoading } = useSession();

  // Filters State
  const [statusFilter, setStatusFilter] = useState("all");
  const [opTypeFilter, setOpTypeFilter] = useState("");
  const [nodeFilter, setNodeFilter] = useState("");

  // Data & Loading State
  const [operations, setOperations] = useState<OperationItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Selected Item for Detail Dialog
  const [selectedOp, setSelectedOp] = useState<OperationItem | null>(null);
  const [retryingId, setRetryingId] = useState<string | null>(null);
  const [isApplyAllOpen, setIsApplyAllOpen] = useState(false);

  const fetchOperations = async () => {
    try {
      setIsLoading(true);
      setError(null);

      const items = await listOperations({
        status: statusFilter,
        kind: opTypeFilter.trim() || undefined,
        nodeId: nodeFilter.trim() || undefined,
      });
      setOperations(items);
    } catch {
      setError(t("common.error"));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchOperations();
  }, [statusFilter]);

  const handleApplyFilters = (e: React.FormEvent) => {
    e.preventDefault();
    fetchOperations();
  };

  const handleResetFilters = () => {
    setStatusFilter("all");
    setOpTypeFilter("");
    setNodeFilter("");
  };

  const handleRetry = async (opId: string) => {
    try {
      setRetryingId(opId);
      const operation = await retryOperation(opId);
      toast({
        title: t("operations.retrySuccess"),
        variant: "default",
      });
      if (selectedOp && selectedOp.id === opId) {
        setSelectedOp(operation);
      }
      fetchOperations();
    } catch {
      toast({
        title: t("common.error"),
        variant: "destructive",
      });
    } finally {
      setRetryingId(null);
    }
  };

  const renderStatusBadge = (status: OperationItem["status"]) => {
    switch (status) {
      case "succeeded":
        return (
          <Badge variant="success" className="gap-1 text-[11px] py-0 px-2 font-medium">
            <CheckCircle2 className="h-3 w-3" />
            {t("operations.status.succeeded")}
          </Badge>
        );
      case "failed":
        return (
          <Badge variant="destructive" className="gap-1 text-[11px] py-0 px-2 font-medium">
            <XCircle className="h-3 w-3" />
            {t("operations.status.failed")}
          </Badge>
        );
      case "retryable":
        return (
          <Badge variant="outline" className="gap-1 text-[11px] py-0 px-2 text-amber-500 border-amber-500 font-medium">
            <AlertTriangle className="h-3 w-3" />
            {t("operations.status.retryable")}
          </Badge>
        );
      case "dispatched":
        return (
          <Badge variant="secondary" className="gap-1 text-[11px] py-0 px-2 font-medium text-sky-500">
            <Activity className="h-3 w-3 animate-pulse" />
            {t("operations.status.dispatched")}
          </Badge>
        );
      case "pending":
      default:
        return (
          <Badge variant="secondary" className="gap-1 text-[11px] py-0 px-2 font-medium text-muted-foreground">
            <Clock className="h-3 w-3" />
            {t("operations.status.pending")}
          </Badge>
        );
    }
  };

  if (isSessionLoading) {
    return (
      <div className="py-12 text-center text-sm text-muted-foreground">
        {t("common.loading")}
      </div>
    );
  }

  // Permission Guard
  if (!hasPermission("admin.operations.read")) {
    return (
      <div className="p-8 text-center border rounded-lg bg-card">
        <p className="text-sm text-muted-foreground">{t("common.noPermission")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t("operations.title")}</h1>
          <p className="text-sm text-muted-foreground">{t("operations.desc")}</p>
        </div>
        <div className="flex items-center gap-2">
          {CLUSTER_ROLLOUT && hasPermission("admin.operations.write") && (
              <Button
              size="sm"
              onClick={() => setIsApplyAllOpen(true)}
              data-testid="btn-open-apply-all"
              className="gap-1.5 text-xs"
            >
              <Layers className="h-3.5 w-3.5" />
              {t("operations.applyAll.openBtn")}
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={fetchOperations} className="gap-1.5 text-xs">
            <RotateCcw className="h-3.5 w-3.5" />
            {t("common.reload")}
          </Button>
        </div>
      </div>

      {/* Filter Card */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Filter className="h-4 w-4 text-primary" />
            {t("operations.filters.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleApplyFilters} className="space-y-4">
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="statusFilter">
                  {t("operations.filters.status")}
                </label>
                <select
                  id="statusFilter"
                  className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  value={statusFilter}
                  onChange={(e) => setStatusFilter(e.target.value)}
                >
                  <option value="all">{t("operations.status.all")}</option>
                  <option value="pending">{t("operations.status.pending")}</option>
                  <option value="dispatched">{t("operations.status.dispatched")}</option>
                  <option value="succeeded">{t("operations.status.succeeded")}</option>
                  <option value="failed">{t("operations.status.failed")}</option>
                  <option value="retryable">{t("operations.status.retryable")}</option>
                </select>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="opTypeFilter">
                  {t("operations.filters.opType")}
                </label>
                <Input
                  id="opTypeFilter"
                  placeholder={t("operations.filters.opTypePlaceholder")}
                  value={opTypeFilter}
                  onChange={(e) => setOpTypeFilter(e.target.value)}
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="nodeFilter">
                  {t("operations.filters.node")}
                </label>
                <Input
                  id="nodeFilter"
                  placeholder={t("operations.filters.nodePlaceholder")}
                  value={nodeFilter}
                  onChange={(e) => setNodeFilter(e.target.value)}
                />
              </div>
            </div>

            <div className="flex gap-2 justify-end pt-1">
              <Button type="button" variant="outline" size="sm" onClick={handleResetFilters} className="gap-1.5 text-xs">
                <RotateCcw className="h-3.5 w-3.5" />
                {t("operations.filters.reset")}
              </Button>
              <Button type="submit" size="sm" className="gap-1.5 text-xs">
                <Filter className="h-3.5 w-3.5" />
                {t("operations.filters.apply")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Operations Table Card */}
      <Card>
        <CardContent className="p-0">
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[180px]">{t("operations.table.id")}</TableHead>
                  <TableHead>{t("operations.table.type")}</TableHead>
                  <TableHead className="w-[120px]">{t("operations.table.status")}</TableHead>
                  <TableHead>{t("operations.table.node")}</TableHead>
                  <TableHead className="w-[90px]">{t("operations.table.retries")}</TableHead>
                  <TableHead>{t("operations.table.time")}</TableHead>
                  <TableHead className="text-right">{t("operations.table.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {isLoading ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-muted-foreground text-sm">
                      {t("common.loading")}
                    </TableCell>
                  </TableRow>
                ) : error ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-destructive text-sm">
                      {error}
                    </TableCell>
                  </TableRow>
                ) : operations.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-muted-foreground text-sm">
                      {t("operations.empty")}
                    </TableCell>
                  </TableRow>
                ) : (
                  operations.map((op) => {
                    const canRetry = op.status === "failed" || op.status === "retryable";
                    return (
                      <TableRow key={op.id}>
                        <TableCell>
                          <span className="font-mono text-xs text-muted-foreground" title={op.id}>
                            {op.id.slice(0, 8)}...{op.id.slice(-4)}
                          </span>
                        </TableCell>
                        <TableCell>
                          <span className="font-mono text-xs font-semibold px-2 py-0.5 rounded bg-muted/60 text-foreground flex items-center gap-1.5 w-fit">
                            <Layers className="h-3 w-3 text-primary" />
                            {op.op_type}
                          </span>
                        </TableCell>
                        <TableCell>{renderStatusBadge(op.status)}</TableCell>
                        <TableCell>
                          <span className="font-mono text-xs text-muted-foreground">
                            {op.node_name || op.node_id.slice(0, 8)}
                          </span>
                        </TableCell>
                        <TableCell className="text-xs font-mono">{op.retries_count}</TableCell>
                        <TableCell className="text-xs text-muted-foreground whitespace-nowrap">
                          {formatDate(new Date(op.created_at))}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex items-center justify-end gap-1.5">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => setSelectedOp(op)}
                              className="h-8 px-2 text-xs gap-1"
                            >
                              <Code2 className="h-3.5 w-3.5" />
                              {t("operations.table.viewDetails")}
                            </Button>
                            {hasPermission("admin.operations.write") && canRetry && (
                              <Button
                                variant="outline"
                                size="sm"
                                disabled={retryingId === op.id}
                                onClick={() => handleRetry(op.id)}
                                className="h-8 px-2.5 text-xs gap-1 text-primary hover:bg-primary/10 border-primary/30"
                              >
                                <RotateCcw className="h-3.5 w-3.5" />
                                {retryingId === op.id ? t("operations.table.retrying") : t("operations.table.retry")}
                              </Button>
                            )}
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

      {/* Operation Detail Dialog */}
      <OperationDetailDialog
        operation={selectedOp}
        open={!!selectedOp}
        onOpenChange={(open) => !open && setSelectedOp(null)}
        onRetry={handleRetry}
        isRetrying={!!retryingId && retryingId === selectedOp?.id}
      />

      {/* Apply to All Dialog (T16.06) */}
      <ApplyToAllDialog
        open={isApplyAllOpen}
        onOpenChange={setIsApplyAllOpen}
        onCompleted={fetchOperations}
      />
    </div>
  );
}
