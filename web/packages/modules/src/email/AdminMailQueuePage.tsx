import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Button,
  Badge,
  useToast,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import {
  Send,
  Trash2,
  RefreshCw,
  ShieldCheck,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Inbox,
} from "lucide-react";
import { AdminMailControl } from "./AdminMailControl";
import { MailQueueItem, DeliverabilityCheck } from "./types";

export function AdminMailQueuePage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"queue" | "deliverability">("queue");
  const [queueItems, setQueueItems] = useState<MailQueueItem[]>([]);
  const [deliverability, setDeliverability] = useState<DeliverabilityCheck[]>([]);
  const [loading, setLoading] = useState(true);
  const [flushing, setFlushing] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<MailQueueItem | null>(null);

  const loadData = useCallback(async () => {
    try {
      setLoading(true);
      const [qRes, dRes] = await Promise.all([
        apiFetch<MailQueueItem[]>("/api/v1/admin/email/queue"),
        apiFetch<DeliverabilityCheck[]>("/api/v1/admin/email/deliverability"),
      ]);
      setQueueItems(qRes);
      setDeliverability(dRes);
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("email.failedToLoadQueue", "Failed to load mail queue data"),
        variant: "destructive",
      });
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleFlushQueue = async () => {
    setFlushing(true);
    try {
      await apiFetch("/api/v1/admin/email/queue/flush", { method: "POST" });
      toast({
        title: t("email.flushSuccessTitle", "Queue flush initiated"),
        description: t("email.flushSuccessDesc", "Outbound mail delivery retry has been triggered."),
        variant: "success",
      });
      loadData();
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("email.flushFailed", "Failed to flush queue"),
        variant: "destructive",
      });
    } finally {
      setFlushing(false);
    }
  };

  const handleDeleteItem = async () => {
    if (!deleteTarget) return;
    try {
      await apiFetch(`/api/v1/admin/email/queue/${deleteTarget.id}`, { method: "DELETE" });
      toast({
        title: t("email.itemDeletedTitle", "Message deleted"),
        description: t("email.itemDeletedDesc", "Message removed from mail queue."),
        variant: "success",
      });
      setDeleteTarget(null);
      loadData();
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("email.deleteFailed", "Failed to remove queue message"),
        variant: "destructive",
      });
    }
  };

  const renderBadge = (status: "pass" | "neutral" | "fail" | "missing") => {
    switch (status) {
      case "pass":
        return (
          <Badge variant="success" className="gap-1">
            <CheckCircle2 className="h-3 w-3" /> PASS
          </Badge>
        );
      case "neutral":
        return (
          <Badge variant="secondary" className="gap-1">
            <AlertTriangle className="h-3 w-3" /> NEUTRAL
          </Badge>
        );
      case "fail":
        return (
          <Badge variant="destructive" className="gap-1">
            <XCircle className="h-3 w-3" /> FAIL
          </Badge>
        );
      case "missing":
      default:
        return (
          <Badge variant="outline" className="gap-1 text-muted-foreground">
            <AlertTriangle className="h-3 w-3" /> MISSING
          </Badge>
        );
    }
  };

  return (
    <div className="space-y-6" data-testid="admin-mail-queue-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="mail-queue-title">
            {t("email.adminMailTitle", "Mail Server & Deliverability")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t("email.adminMailSubtitle", "Monitor Postfix/Exim mail queue, retry deferred mail, and check domain SPF/DKIM/DMARC.")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {activeTab === "queue" && (
            <Button
              onClick={handleFlushQueue}
              disabled={flushing || loading || queueItems.length === 0}
              className="gap-2"
              data-testid="flush-queue-btn"
            >
              <Send className={`h-4 w-4 ${flushing ? "animate-spin" : ""}`} />
              {flushing ? t("email.flushingBtn", "Flushing...") : t("email.flushQueueBtn", "Flush Queue")}
            </Button>
          )}

          <Button
            variant="outline"
            size="sm"
            onClick={loadData}
            disabled={loading}
            className="gap-2"
            data-testid="refresh-mail-queue-btn"
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Tabs */}
      <div className="border-b flex gap-4">
        <button
          onClick={() => setActiveTab("queue")}
          data-testid="tab-admin-queue"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "queue"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Inbox className="h-4 w-4" />
          {t("email.mailQueueTab", "Mail Queue")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs">
            {queueItems.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("deliverability")}
          data-testid="tab-admin-deliverability"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "deliverability"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <ShieldCheck className="h-4 w-4" />
          {t("email.deliverabilityTab", "Deliverability & DNS Auth")}
        </button>
      </div>

      {/* Queue View */}
      {activeTab === "queue" && (
        <div className="rounded-lg border bg-card" data-testid="queue-table-section">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("email.queueIdCol", "Queue ID")}</TableHead>
                <TableHead>{t("email.senderCol", "Sender")}</TableHead>
                <TableHead>{t("email.recipientCol", "Recipient")}</TableHead>
                <TableHead>{t("common.status", "Status")}</TableHead>
                <TableHead>{t("email.retriesCol", "Retries")}</TableHead>
                <TableHead>{t("email.queuedAtCol", "Queued At")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8 text-muted-foreground">
                    {t("common.loading", "Loading mail queue...")}
                  </TableCell>
                </TableRow>
              ) : queueItems.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8 text-muted-foreground">
                    {t("email.emptyQueue", "Mail queue is empty. All messages delivered successfully.")}
                  </TableCell>
                </TableRow>
              ) : (
                queueItems.map((item) => (
                  <TableRow key={item.id} data-testid={`queue-row-${item.id}`}>
                    <TableCell className="font-mono text-xs font-semibold">
                      {item.id}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {item.sender || "<empty>"}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {item.recipient}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          item.status === "queued"
                            ? "secondary"
                            : item.status === "deferred"
                            ? "outline"
                            : "destructive"
                        }
                      >
                        {item.status}
                      </Badge>
                      {item.error_message && (
                        <p className="text-[11px] text-destructive mt-0.5 max-w-xs truncate" title={item.error_message}>
                          {item.error_message}
                        </p>
                      )}
                    </TableCell>
                    <TableCell className="text-xs font-mono">
                      {item.retries}
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(item.queued_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setDeleteTarget(item)}
                        className="text-destructive hover:bg-destructive/10"
                        data-testid={`delete-queue-item-${item.id}`}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Deliverability View */}
      {activeTab === "deliverability" && (
        <div className="space-y-4" data-testid="deliverability-section">
          {loading ? (
            <div className="p-8 text-center text-muted-foreground">
              {t("common.loading", "Checking domain deliverability...")}
            </div>
          ) : deliverability.length === 0 ? (
            <div className="p-8 text-center text-muted-foreground border rounded-lg">
              {t("email.noDeliverabilityData", "No deliverability reports available.")}
            </div>
          ) : (
            deliverability.map((item) => (
              <div
                key={item.domain}
                className="border rounded-lg p-5 bg-card space-y-4 shadow-sm"
                data-testid={`deliverability-card-${item.domain}`}
              >
                <div className="flex flex-col sm:flex-row sm:items-center justify-between pb-3 border-b gap-2">
                  <div>
                    <h3 className="text-lg font-bold font-mono">@{item.domain}</h3>
                    <p className="text-xs text-muted-foreground">
                      {t("email.deliverabilityDomainDesc", "DNS Authentication & Blacklist Status")}
                    </p>
                  </div>
                  <div>
                    {item.rbl_blacklisted ? (
                      <Badge variant="destructive" className="gap-1">
                        <AlertTriangle className="h-3 w-3" />
                        {t("email.rblBlacklisted", "Listed on RBL Blacklists")}
                      </Badge>
                    ) : (
                      <Badge variant="success" className="gap-1">
                        <CheckCircle2 className="h-3 w-3" />
                        {t("email.rblClean", "Clean IP Reputation")}
                      </Badge>
                    )}
                  </div>
                </div>

                {/* Status Badges */}
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
                  <div className="p-3 bg-muted/40 rounded-lg">
                    <span className="text-xs text-muted-foreground block mb-1 font-semibold">SPF</span>
                    {renderBadge(item.spf_status)}
                  </div>
                  <div className="p-3 bg-muted/40 rounded-lg">
                    <span className="text-xs text-muted-foreground block mb-1 font-semibold">DKIM</span>
                    {renderBadge(item.dkim_status)}
                  </div>
                  <div className="p-3 bg-muted/40 rounded-lg">
                    <span className="text-xs text-muted-foreground block mb-1 font-semibold">DMARC</span>
                    {renderBadge(item.dmarc_status)}
                  </div>
                  <div className="p-3 bg-muted/40 rounded-lg">
                    <span className="text-xs text-muted-foreground block mb-1 font-semibold">MX Record</span>
                    {renderBadge(item.mx_status)}
                  </div>
                </div>

                {/* Recommendations */}
                {item.recommendations.length > 0 && (
                  <div className="pt-2">
                    <h4 className="text-xs font-semibold text-muted-foreground mb-2">
                      {t("email.recommendationsTitle", "Deliverability Recommendations")}:
                    </h4>
                    <ul className="list-disc list-inside space-y-1 text-xs text-foreground">
                      {item.recommendations.map((rec, idx) => (
                        <li key={idx}>{rec}</li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      )}

      {/* Delete Confirmation */}
      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <DialogTitle>{t("email.deleteQueueMsgTitle", "Delete Queued Message")}</DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "email.deleteQueueMsgDesc",
                  "Are you sure you want to drop message {{id}} to {{recipient}} from the mail queue?",
                  { id: deleteTarget.id, recipient: deleteTarget.recipient }
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={() => setDeleteTarget(null)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button variant="destructive" onClick={handleDeleteItem}>
                {t("common.delete", "Delete")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
      <AdminMailControl />
    </div>
  );
}
