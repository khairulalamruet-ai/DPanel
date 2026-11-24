import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import {
  Activity,
  RotateCcw,
  Copy,
  Check,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Clock,
  Layers,
  Code2,
} from "lucide-react";
import type { OperationItem } from "./types";

interface OperationDetailDialogProps {
  operation: OperationItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRetry?: (opId: string) => Promise<void>;
  isRetrying?: boolean;
}

export function OperationDetailDialog({
  operation,
  open,
  onOpenChange,
  onRetry,
  isRetrying = false,
}: OperationDetailDialogProps) {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  if (!operation) return null;

  const copyData = (key: string, data: unknown) => {
    navigator.clipboard?.writeText(JSON.stringify(data, null, 2));
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 2000);
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

  const canRetry = operation.status === "failed" || operation.status === "retryable";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Layers className="h-5 w-5 text-primary" />
              <DialogTitle className="font-mono">{t("operations.modal.title")} — {operation.op_type}</DialogTitle>
            </div>
            {renderStatusBadge(operation.status)}
          </div>
          <DialogDescription className="font-mono text-xs pt-0.5">
            ID: {operation.id}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* Metadata Grid */}
          <div className="grid grid-cols-2 gap-3 p-3.5 rounded-lg border bg-muted/30 text-xs">
            <div>
              <span className="font-semibold text-muted-foreground">{t("operations.modal.idempotencyKey")}:</span>
              <p className="font-mono mt-0.5 break-all">{operation.idempotency_key}</p>
            </div>
            <div>
              <span className="font-semibold text-muted-foreground">{t("operations.modal.nodeId")}:</span>
              <p className="font-mono mt-0.5">{operation.node_name || operation.node_id}</p>
            </div>
            <div>
              <span className="font-semibold text-muted-foreground">{t("operations.modal.attempts")}:</span>
              <p className="font-mono mt-0.5">{operation.retries_count}</p>
            </div>
            <div>
              <span className="font-semibold text-muted-foreground">{t("operations.table.time")}:</span>
              <p className="mt-0.5">{formatDate(new Date(operation.created_at))}</p>
            </div>
            {operation.tenant_id && (
              <div className="col-span-2">
                <span className="font-semibold text-muted-foreground">{t("operations.modal.tenantId")}:</span>
                <p className="font-mono mt-0.5">{operation.tenant_id}</p>
              </div>
            )}
          </div>

          {/* Error Section (if any) */}
          {operation.error && (
            <div className="p-3.5 rounded-lg border border-destructive/30 bg-destructive/5 space-y-2 text-xs">
              <div className="flex items-center justify-between text-destructive font-semibold">
                <div className="flex items-center gap-1.5">
                  <AlertTriangle className="h-4 w-4" />
                  <span>{t("operations.modal.error")} ({operation.error.code})</span>
                </div>
              </div>
              <p className="font-mono text-destructive">{operation.error.message}</p>
              {operation.error.details && (
                <pre className="p-2.5 rounded bg-zinc-950 text-zinc-100 font-mono text-[11px] overflow-x-auto border">
                  {JSON.stringify(operation.error.details, null, 2)}
                </pre>
              )}
            </div>
          )}

          {/* Payload Section */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold uppercase text-muted-foreground flex items-center gap-1.5">
                <Code2 className="h-3.5 w-3.5 text-primary" />
                {t("operations.modal.payload")}
              </label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => copyData("payload", operation.payload)}
                className="gap-1 text-xs h-7 px-2"
              >
                {copiedKey === "payload" ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                {copiedKey === "payload" ? t("operations.modal.copied") : t("operations.modal.copyJson")}
              </Button>
            </div>
            <pre className="p-3 rounded-lg bg-muted/60 font-mono text-xs overflow-x-auto border max-h-48 select-all">
              {JSON.stringify(operation.payload, null, 2)}
            </pre>
          </div>

          {/* Output Section (if succeeded) */}
          {operation.output && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-semibold uppercase text-muted-foreground flex items-center gap-1.5">
                  <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
                  {t("operations.modal.output")}
                </label>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => copyData("output", operation.output)}
                  className="gap-1 text-xs h-7 px-2"
                >
                  {copiedKey === "output" ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                  {copiedKey === "output" ? t("operations.modal.copied") : t("operations.modal.copyJson")}
                </Button>
              </div>
              <pre className="p-3 rounded-lg bg-muted/60 font-mono text-xs overflow-x-auto border max-h-48 select-all">
                {JSON.stringify(operation.output, null, 2)}
              </pre>
            </div>
          )}
        </div>

        <DialogFooter className="gap-2">
          {canRetry && onRetry && (
            <Button
              variant="default"
              disabled={isRetrying}
              onClick={() => onRetry(operation.id)}
              className="gap-1.5 text-xs"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              {isRetrying ? t("operations.table.retrying") : t("operations.modal.retryOp")}
            </Button>
          )}
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("operations.modal.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
