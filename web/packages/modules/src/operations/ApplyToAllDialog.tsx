import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Layers,
  Play,
  RotateCcw,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Clock,
  Server,
  Zap,
  ShieldAlert,
} from "lucide-react";
import {
  ApplyToAllPayload,
  ApplyToAllExecutionResult,
  RollingNodeProgress,
} from "./types";

export interface ApplyToAllDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCompleted?: () => void;
}

export function ApplyToAllDialog({
  open,
  onOpenChange,
  onCompleted,
}: ApplyToAllDialogProps) {
  const { t } = useTranslation();

  const [opType, setOpType] = useState<string>("SyncSettings");
  const [batchSize, setBatchSize] = useState<number>(2);
  const [delayMs, setDelayMs] = useState<number>(300);
  const [stopOnFailure, setStopOnFailure] = useState<boolean>(true);
  const [simulateFailNode, setSimulateFailNode] = useState<string>("");

  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [result, setResult] = useState<ApplyToAllExecutionResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleStart = async () => {
    try {
      setIsRunning(true);
      setError(null);
      setResult(null);

      const payload: ApplyToAllPayload = {
        op_type: opType,
        batch_size: batchSize,
        delay_ms: delayMs,
        stop_on_failure: stopOnFailure,
        simulate_fail_node_id: simulateFailNode || null,
      };

      const res = await apiFetch<ApplyToAllExecutionResult>(
        "/api/v1/admin/cluster/apply-all",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }
      );

      setResult(res);
      if (onCompleted) {
        onCompleted();
      }
    } catch (err: any) {
      setError(err?.message || "Failed to execute rolling apply");
    } finally {
      setIsRunning(false);
    }
  };

  const handleReset = () => {
    setResult(null);
    setError(null);
    setSimulateFailNode("");
  };

  const renderNodeStatusBadge = (status: RollingNodeProgress["status"]) => {
    switch (status) {
      case "succeeded":
        return (
          <Badge
            variant="secondary"
            className="text-[10px] bg-emerald-500/10 text-emerald-600 border border-emerald-500/20 font-medium flex items-center gap-1"
          >
            <CheckCircle2 className="h-3 w-3" />
            {t("operations.applyAll.status.succeeded")}
          </Badge>
        );
      case "failed":
        return (
          <Badge
            variant="destructive"
            className="text-[10px] font-medium flex items-center gap-1"
          >
            <XCircle className="h-3 w-3" />
            {t("operations.applyAll.status.failed")}
          </Badge>
        );
      case "skipped":
        return (
          <Badge
            variant="outline"
            className="text-[10px] bg-muted/50 text-muted-foreground font-medium flex items-center gap-1"
          >
            <Clock className="h-3 w-3" />
            {t("operations.applyAll.status.skipped")}
          </Badge>
        );
      case "running":
        return (
          <Badge
            variant="secondary"
            className="text-[10px] bg-blue-500/10 text-blue-600 border border-blue-500/20 font-medium flex items-center gap-1"
          >
            <RotateCcw className="h-3 w-3 animate-spin" />
            {t("operations.applyAll.status.running")}
          </Badge>
        );
      default:
        return (
          <Badge variant="outline" className="text-[10px] font-medium">
            {t("operations.applyAll.status.pending")}
          </Badge>
        );
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-3xl max-h-[90vh] flex flex-col p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Layers className="h-5 w-5" />
            <DialogTitle>{t("operations.applyAll.dialogTitle")}</DialogTitle>
          </div>
          <DialogDescription>{t("operations.applyAll.dialogDesc")}</DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pr-1 my-3">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* Operation Configuration Form */}
          {!result && (
            <div className="space-y-4">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                {/* Operation Type */}
                <div className="space-y-1">
                  <label htmlFor="apply-op-type" className="text-xs font-medium text-foreground">
                    {t("operations.applyAll.opTypeLabel")}
                  </label>
                  <select
                    id="apply-op-type"
                    data-testid="select-apply-op-type"
                    value={opType}
                    onChange={(e) => setOpType(e.target.value)}
                    disabled={isRunning}
                    className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm text-foreground"
                  >
                    <option value="SyncSettings">{t("operations.applyAll.op.syncSettings")}</option>
                    <option value="UpdateVhostTemplates">{t("operations.applyAll.op.updateVhost")}</option>
                    <option value="RestartWebService">{t("operations.applyAll.op.restartWeb")}</option>
                    <option value="ApplySecurityPolicy">{t("operations.applyAll.op.applySecurity")}</option>
                  </select>
                </div>

                {/* Target Scope */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-foreground">
                    {t("operations.applyAll.targetScopeLabel")}
                  </label>
                  <div className="h-9 px-3 flex items-center rounded-md border bg-muted/30 text-xs font-medium text-foreground">
                    <Server className="h-3.5 w-3.5 mr-1.5 text-primary" />
                    <span>{t("operations.applyAll.allNodesScope")}</span>
                  </div>
                </div>
              </div>

              {/* Execution Policy Controls */}
              <div className="p-3.5 rounded-lg border bg-muted/20 space-y-3">
                <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground block">
                  {t("operations.applyAll.policySection")}
                </span>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div className="space-y-1">
                    <label htmlFor="batch-size" className="text-xs font-medium text-foreground">
                      {t("operations.applyAll.batchSizeLabel")}
                    </label>
                    <select
                      id="batch-size"
                      data-testid="select-batch-size"
                      value={batchSize}
                      onChange={(e) => setBatchSize(Number(e.target.value))}
                      disabled={isRunning}
                      className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
                    >
                      <option value={1}>{t("operations.applyAll.batch1")}</option>
                      <option value={2}>{t("operations.applyAll.batch2")}</option>
                      <option value={5}>{t("operations.applyAll.batch5")}</option>
                    </select>
                  </div>

                  <div className="space-y-1">
                    <label htmlFor="rate-delay" className="text-xs font-medium text-foreground">
                      {t("operations.applyAll.delayLabel")}
                    </label>
                    <select
                      id="rate-delay"
                      value={delayMs}
                      onChange={(e) => setDelayMs(Number(e.target.value))}
                      disabled={isRunning}
                      className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
                    >
                      <option value={100}>100 ms</option>
                      <option value={300}>300 ms</option>
                      <option value={1000}>1,000 ms (1s)</option>
                    </select>
                  </div>
                </div>

                {/* Circuit Breaker Toggle */}
                <div className="flex items-center justify-between pt-1 border-t">
                  <div>
                    <span className="text-xs font-medium text-foreground block">
                      {t("operations.applyAll.stopOnFailureLabel")}
                    </span>
                    <span className="text-[11px] text-muted-foreground">
                      {t("operations.applyAll.stopOnFailureDesc")}
                    </span>
                  </div>
                  <input
                    type="checkbox"
                    id="stop-on-failure-toggle"
                    data-testid="checkbox-stop-on-failure"
                    checked={stopOnFailure}
                    onChange={(e) => setStopOnFailure(e.target.checked)}
                    disabled={isRunning}
                    className="h-4 w-4 rounded border-gray-300 text-primary focus:ring-primary"
                  />
                </div>
              </div>

              {/* Simulation option for circuit breaker test */}
              <div className="p-3 rounded-lg border border-dashed bg-card space-y-1.5">
                <label htmlFor="simulate-fail" className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <ShieldAlert className="h-3.5 w-3.5 text-amber-500" />
                  <span>{t("operations.applyAll.simulateFailLabel")}</span>
                </label>
                <select
                  id="simulate-fail"
                  data-testid="select-simulate-fail"
                  value={simulateFailNode}
                  onChange={(e) => setSimulateFailNode(e.target.value)}
                  disabled={isRunning}
                  className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
                >
                  <option value="">{t("operations.applyAll.noSimulatedFail")}</option>
                  <option value="node-04">node-worker-04 (Fail at Batch 2)</option>
                  <option value="node-07">node-worker-07 (Fail at Batch 4)</option>
                </select>
              </div>
            </div>
          )}

          {/* Live Rolling Progress & Results View (DoD T16.06) */}
          {result && (
            <div className="space-y-4" data-testid="rolling-apply-results">
              {/* Status Header Banner */}
              <div
                className={`p-4 rounded-lg border flex items-center justify-between ${
                  result.status === "succeeded"
                    ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-900 dark:text-emerald-300"
                    : "bg-destructive/10 border-destructive/30 text-destructive"
                }`}
              >
                <div className="flex items-center gap-3">
                  {result.status === "succeeded" ? (
                    <CheckCircle2 className="h-6 w-6 text-emerald-600 shrink-0" />
                  ) : (
                    <AlertTriangle className="h-6 w-6 text-destructive shrink-0" />
                  )}
                  <div>
                    <span className="font-bold text-sm block" data-testid="rolling-status-title">
                      {result.status === "succeeded"
                        ? t("operations.applyAll.completedTitle")
                        : t("operations.applyAll.haltedTitle")}
                    </span>
                    <span className="text-xs text-muted-foreground">{result.message}</span>
                  </div>
                </div>
                <Badge
                  variant="outline"
                  data-testid="rolling-result-badge"
                  className={`text-xs px-2.5 py-1 font-mono uppercase ${
                    result.status === "succeeded"
                      ? "border-emerald-500/40 text-emerald-700 bg-emerald-500/10 font-bold"
                      : "border-destructive text-destructive bg-destructive/10 font-bold"
                  }`}
                >
                  {result.status}
                </Badge>
              </div>

              {/* Progress Summary Cards */}
              <div className="grid grid-cols-4 gap-2 text-center text-xs">
                <div className="p-2.5 rounded-lg border bg-muted/20">
                  <span className="text-muted-foreground block text-[11px]">
                    {t("operations.applyAll.totalNodes")}
                  </span>
                  <span className="font-bold text-base" data-testid="metric-total-nodes">
                    {result.total_nodes}
                  </span>
                </div>
                <div className="p-2.5 rounded-lg border bg-muted/20">
                  <span className="text-muted-foreground block text-[11px]">
                    {t("operations.applyAll.succeededNodes")}
                  </span>
                  <span className="font-bold text-base text-emerald-600" data-testid="metric-succeeded-nodes">
                    {result.succeeded_nodes}
                  </span>
                </div>
                <div className="p-2.5 rounded-lg border bg-muted/20">
                  <span className="text-muted-foreground block text-[11px]">
                    {t("operations.applyAll.failedNodes")}
                  </span>
                  <span className="font-bold text-base text-destructive">
                    {result.failed_nodes}
                  </span>
                </div>
                <div className="p-2.5 rounded-lg border bg-muted/20">
                  <span className="text-muted-foreground block text-[11px]">
                    {t("operations.applyAll.skippedNodes")}
                  </span>
                  <span className="font-bold text-base text-amber-600" data-testid="metric-skipped-nodes">
                    {result.skipped_nodes}
                  </span>
                </div>
              </div>

              {/* Per-Node Progress Table */}
              <div className="rounded-lg border overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-[15%]">{t("operations.applyAll.table.batch")}</TableHead>
                      <TableHead className="w-[30%]">{t("operations.applyAll.table.node")}</TableHead>
                      <TableHead className="w-[20%]">{t("operations.applyAll.table.status")}</TableHead>
                      <TableHead className="w-[15%]">{t("operations.applyAll.table.latency")}</TableHead>
                      <TableHead className="w-[20%]">{t("operations.applyAll.table.output")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {result.node_results.map((nr) => (
                      <TableRow key={nr.node_id} data-testid={`node-progress-row-${nr.node_id}`}>
                        <TableCell>
                          <Badge variant="outline" className="font-mono text-[10px]">
                            Batch {nr.batch_index + 1}
                          </Badge>
                        </TableCell>
                        <TableCell>
                          <span className="font-medium text-xs text-foreground font-mono">
                            {nr.node_name}
                          </span>
                        </TableCell>
                        <TableCell>{renderNodeStatusBadge(nr.status)}</TableCell>
                        <TableCell>
                          <span className="text-xs text-muted-foreground font-mono">
                            {nr.duration_ms ? `${nr.duration_ms} ms` : "—"}
                          </span>
                        </TableCell>
                        <TableCell>
                          <span className="text-xs text-muted-foreground truncate max-w-[180px] block">
                            {nr.error || nr.output || "—"}
                          </span>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            </div>
          )}
        </div>

        <DialogFooter className="pt-3 border-t gap-2">
          {result ? (
            <>
              <Button type="button" variant="outline" onClick={handleReset}>
                {t("operations.applyAll.reconfigureBtn")}
              </Button>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t("common.close")}
              </Button>
            </>
          ) : (
            <>
              <Button
                type="button"
                variant="outline"
                disabled={isRunning}
                onClick={() => onOpenChange(false)}
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="button"
                disabled={isRunning}
                data-testid="btn-start-rolling-apply"
                onClick={handleStart}
                className="gap-2"
              >
                {isRunning ? (
                  <RotateCcw className="h-4 w-4 animate-spin" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                {isRunning
                  ? t("operations.applyAll.executing")
                  : t("operations.applyAll.startBtn")}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
