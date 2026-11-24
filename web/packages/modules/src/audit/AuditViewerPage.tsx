import React, { useState, useEffect } from "react";
import {
  Button,
  Input,
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import { api } from "@builtpanel/api";
import { useSession } from "@builtpanel/auth";
import {
  FileText,
  Filter,
  RotateCcw,
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Code2,
} from "lucide-react";

interface AuditItem {
  id: string;
  at: string;
  action: string;
  outcome: "success" | "failure" | "denied" | string;
  actor_id?: string | null;
  actor_ip?: string | null;
  correlation_id?: string | null;
  node_id?: string | null;
  target_type?: string | null;
  target_id?: string | null;
  tenant_id?: string | null;
  metadata: Record<string, unknown>;
}

export function AuditViewerPage() {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();
  const { hasPermission, isLoading: isSessionLoading } = useSession();

  // Filters State
  const [actionFilter, setActionFilter] = useState("");
  const [actorFilter, setActorFilter] = useState("");
  const [outcomeFilter, setOutcomeFilter] = useState<string>("all");
  const [targetTypeFilter, setTargetTypeFilter] = useState("");

  // Pagination State
  const [limit, setLimit] = useState(10);
  const [cursorHistory, setCursorHistory] = useState<string[]>([]);
  const [currentCursor, setCurrentCursor] = useState<string | undefined>(undefined);
  const [nextCursor, setNextCursor] = useState<string | null>(null);

  // Data & Loading State
  const [items, setItems] = useState<AuditItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Selected Item for Metadata Dialog
  const [selectedItem, setSelectedItem] = useState<AuditItem | null>(null);
  const [copiedJson, setCopiedJson] = useState(false);

  const fetchLogs = async (cursor?: string) => {
    try {
      setIsLoading(true);
      setError(null);

      const params: Record<string, any> = {
        limit: String(limit),
      };
      if (cursor) params.cursor = cursor;
      if (actionFilter.trim()) params.action = actionFilter.trim();
      if (actorFilter.trim()) params.actor_id = actorFilter.trim();
      if (outcomeFilter !== "all") params.outcome = outcomeFilter;
      if (targetTypeFilter.trim()) params.target_type = targetTypeFilter.trim();

      const res = await api.GET("/api/v1/admin/audit-logs", {
        params: { query: params },
      });

      if (res.error) {
        if ((res.response as any)?.status === 403) {
          setError(t("common.noPermission"));
          return;
        }
        setError(t("common.error"));
        return;
      }

      if (res.data) {
        setItems(res.data.items as AuditItem[]);
        setNextCursor(res.data.next_cursor || null);
      }
    } catch {
      setError(t("common.error"));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchLogs(currentCursor);
  }, [currentCursor, limit]);

  const handleApplyFilters = (e: React.FormEvent) => {
    e.preventDefault();
    setCursorHistory([]);
    setCurrentCursor(undefined);
    fetchLogs(undefined);
  };

  const handleResetFilters = () => {
    setActionFilter("");
    setActorFilter("");
    setOutcomeFilter("all");
    setTargetTypeFilter("");
    setCursorHistory([]);
    setCurrentCursor(undefined);
  };

  const handleNextPage = () => {
    if (nextCursor) {
      setCursorHistory((prev) => [...prev, currentCursor || ""]);
      setCurrentCursor(nextCursor);
    }
  };

  const handlePrevPage = () => {
    if (cursorHistory.length > 0) {
      const prev = [...cursorHistory];
      const prevCursor = prev.pop();
      setCursorHistory(prev);
      setCurrentCursor(prevCursor ? prevCursor : undefined);
    }
  };

  const copyMetadata = (data: unknown) => {
    const jsonStr = JSON.stringify(data, null, 2);
    navigator.clipboard?.writeText(jsonStr);
    setCopiedJson(true);
    setTimeout(() => setCopiedJson(false), 2000);
  };

  const renderOutcomeBadge = (outcome: string) => {
    switch (outcome) {
      case "success":
        return (
          <Badge variant="success" className="gap-1 text-[11px] py-0 px-2 font-medium">
            <CheckCircle2 className="h-3 w-3" />
            {t("audit.filters.outcomeSuccess")}
          </Badge>
        );
      case "failure":
        return (
          <Badge variant="destructive" className="gap-1 text-[11px] py-0 px-2 font-medium">
            <XCircle className="h-3 w-3" />
            {t("audit.filters.outcomeFailure")}
          </Badge>
        );
      case "denied":
        return (
          <Badge variant="outline" className="gap-1 text-[11px] py-0 px-2 text-amber-500 border-amber-500 font-medium">
            <AlertTriangle className="h-3 w-3" />
            {t("audit.filters.outcomeDenied")}
          </Badge>
        );
      default:
        return (
          <Badge variant="secondary" className="text-[11px] py-0 px-2">
            {outcome}
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

  if (!hasPermission("admin.audit.read")) {
    return (
      <div className="p-8 text-center border rounded-lg bg-card">
        <p className="text-sm text-muted-foreground">{t("common.noPermission")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t("audit.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("audit.desc")}</p>
      </div>

      {/* Filter Card */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Filter className="h-4 w-4 text-primary" />
            {t("audit.filters.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleApplyFilters} className="space-y-4">
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="actionFilter">
                  {t("audit.filters.action")}
                </label>
                <Input
                  id="actionFilter"
                  placeholder={t("audit.filters.actionPlaceholder")}
                  value={actionFilter}
                  onChange={(e) => setActionFilter(e.target.value)}
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="actorFilter">
                  {t("audit.filters.actor")}
                </label>
                <Input
                  id="actorFilter"
                  placeholder={t("audit.filters.actorPlaceholder")}
                  value={actorFilter}
                  onChange={(e) => setActorFilter(e.target.value)}
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="outcomeFilter">
                  {t("audit.filters.outcome")}
                </label>
                <select
                  id="outcomeFilter"
                  className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  value={outcomeFilter}
                  onChange={(e) => setOutcomeFilter(e.target.value)}
                >
                  <option value="all">{t("audit.filters.outcomeAll")}</option>
                  <option value="success">{t("audit.filters.outcomeSuccess")}</option>
                  <option value="failure">{t("audit.filters.outcomeFailure")}</option>
                  <option value="denied">{t("audit.filters.outcomeDenied")}</option>
                </select>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="targetTypeFilter">
                  {t("audit.filters.targetType")}
                </label>
                <Input
                  id="targetTypeFilter"
                  placeholder={t("audit.filters.targetTypePlaceholder")}
                  value={targetTypeFilter}
                  onChange={(e) => setTargetTypeFilter(e.target.value)}
                />
              </div>
            </div>

            <div className="flex gap-2 justify-end pt-1">
              <Button type="button" variant="outline" size="sm" onClick={handleResetFilters} className="gap-1.5 text-xs">
                <RotateCcw className="h-3.5 w-3.5" />
                {t("audit.filters.reset")}
              </Button>
              <Button type="submit" size="sm" className="gap-1.5 text-xs">
                <Filter className="h-3.5 w-3.5" />
                {t("audit.filters.apply")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Audit Log Table Card */}
      <Card>
        <CardContent className="p-0">
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[180px]">{t("audit.table.timestamp")}</TableHead>
                  <TableHead>{t("audit.table.action")}</TableHead>
                  <TableHead className="w-[110px]">{t("audit.table.outcome")}</TableHead>
                  <TableHead>{t("audit.table.actor")}</TableHead>
                  <TableHead>{t("audit.table.target")}</TableHead>
                  <TableHead>{t("audit.table.ip")}</TableHead>
                  <TableHead className="text-right">{t("audit.table.details")}</TableHead>
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
                ) : items.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-10 text-muted-foreground text-sm">
                      {t("audit.empty")}
                    </TableCell>
                  </TableRow>
                ) : (
                  items.map((item) => (
                    <TableRow key={item.id}>
                      <TableCell className="text-xs text-muted-foreground whitespace-nowrap">
                        {formatDate(new Date(item.at))}
                      </TableCell>
                      <TableCell>
                        <span className="font-mono text-xs font-semibold text-foreground px-1.5 py-0.5 rounded bg-muted/60">
                          {item.action}
                        </span>
                      </TableCell>
                      <TableCell>{renderOutcomeBadge(item.outcome)}</TableCell>
                      <TableCell>
                        <span className="font-mono text-xs text-muted-foreground" title={item.actor_id || "System"}>
                          {item.actor_id ? `${item.actor_id.slice(0, 8)}...` : "system"}
                        </span>
                      </TableCell>
                      <TableCell>
                        {item.target_type ? (
                          <span className="text-xs font-mono">
                            {item.target_type}
                            {item.target_id && `:${item.target_id}`}
                          </span>
                        ) : (
                          <span className="text-xs text-muted-foreground">—</span>
                        )}
                      </TableCell>
                      <TableCell>
                        <span className="font-mono text-xs text-muted-foreground">{item.actor_ip || "—"}</span>
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setSelectedItem(item)}
                          className="gap-1 text-xs h-8 px-2"
                        >
                          <Code2 className="h-3.5 w-3.5" />
                          {t("audit.table.viewMetadata")}
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </div>

          {/* Pagination Controls */}
          <div className="flex flex-col sm:flex-row items-center justify-between gap-4 p-4 border-t">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span>{t("audit.pagination.limit")}:</span>
              <select
                className="h-8 rounded-md border border-input bg-transparent px-2 text-xs"
                value={limit}
                onChange={(e) => {
                  setLimit(Number(e.target.value));
                  setCurrentCursor(undefined);
                  setCursorHistory([]);
                }}
              >
                <option value={10}>10</option>
                <option value={25}>25</option>
                <option value={50}>50</option>
                <option value={100}>100</option>
              </select>
            </div>

            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={handlePrevPage}
                disabled={cursorHistory.length === 0 || isLoading}
                className="gap-1 text-xs h-8"
              >
                <ChevronLeft className="h-3.5 w-3.5" />
                {t("audit.pagination.previous")}
              </Button>
              <span className="text-xs font-medium px-2">
                {t("audit.pagination.page")} {cursorHistory.length + 1}
              </span>
              <Button
                variant="outline"
                size="sm"
                onClick={handleNextPage}
                disabled={!nextCursor || isLoading}
                className="gap-1 text-xs h-8"
              >
                {t("audit.pagination.next")}
                <ChevronRight className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Metadata JSON Modal */}
      <Dialog open={!!selectedItem} onOpenChange={(open) => !open && setSelectedItem(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <div className="flex items-center justify-between">
              <DialogTitle className="flex items-center gap-2 text-base">
                <FileText className="h-5 w-5 text-primary" />
                {t("audit.metadataModal.title")}
              </DialogTitle>
              {selectedItem && renderOutcomeBadge(selectedItem.outcome)}
            </div>
            <DialogDescription className="font-mono text-xs pt-1">
              ID: {selectedItem?.id}
            </DialogDescription>
          </DialogHeader>

          {selectedItem && (
            <div className="space-y-4 pt-2">
              {/* Info Grid */}
              <div className="grid grid-cols-2 gap-3 p-3 rounded-lg bg-muted/40 text-xs">
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.table.action")}:</span>
                  <p className="font-mono mt-0.5">{selectedItem.action}</p>
                </div>
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.table.timestamp")}:</span>
                  <p className="mt-0.5">{formatDate(new Date(selectedItem.at))}</p>
                </div>
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.metadataModal.correlationId")}:</span>
                  <p className="font-mono mt-0.5">{selectedItem.correlation_id || "—"}</p>
                </div>
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.metadataModal.actorIp")}:</span>
                  <p className="font-mono mt-0.5">{selectedItem.actor_ip || "—"}</p>
                </div>
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.table.actor")}:</span>
                  <p className="font-mono mt-0.5">{selectedItem.actor_id || "System"}</p>
                </div>
                <div>
                  <span className="font-semibold text-muted-foreground">{t("audit.metadataModal.nodeId")}:</span>
                  <p className="font-mono mt-0.5">{selectedItem.node_id || "—"}</p>
                </div>
              </div>

              {/* JSON Metadata Viewer */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-semibold uppercase text-muted-foreground flex items-center gap-1.5">
                    <Code2 className="h-3.5 w-3.5 text-primary" />
                    {t("audit.metadataModal.metadata")}
                  </label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => copyMetadata(selectedItem.metadata)}
                    className="gap-1 text-xs h-7 px-2"
                  >
                    {copiedJson ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                    {copiedJson ? t("audit.metadataModal.copied") : t("audit.metadataModal.copyJson")}
                  </Button>
                </div>
                <pre className="p-3.5 rounded-lg bg-muted font-mono text-xs overflow-x-auto border max-h-64 select-all">
                  {JSON.stringify(selectedItem.metadata, null, 2)}
                </pre>
              </div>
            </div>
          )}

          <DialogFooter className="pt-2">
            <Button type="button" variant="ghost" onClick={() => setSelectedItem(null)}>
              {t("audit.metadataModal.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
