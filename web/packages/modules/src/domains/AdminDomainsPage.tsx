import React, { useState, useEffect, useMemo, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Button,
  Badge,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { listAdminDomains, bulkSuspend } from "./adminApi";
import {
  Globe,
  Plus,
  RefreshCw,
  Search,
  ExternalLink,
  Folder,
  ArrowRightLeft,
  ShieldCheck,
  ShieldAlert,
  ShieldOff,
  AlertCircle,
  Clock,
  Trash2,
  AlertTriangle,
  User,
  Server,
  Layers,
  CheckCircle2,
  Ban,
} from "lucide-react";
import {
  AdminDomainItem,
  AdminDomainsResponse,
  DomainType,
  DomainStatus,
  SslStatus,
  DnsStatus,
} from "./types";
import { BulkAddDomainsDialog } from "./BulkAddDomainsDialog";

export function AdminDomainsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [domains, setDomains] = useState<AdminDomainItem[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [perPage, setPerPage] = useState(10);
  const [totalPages, setTotalPages] = useState(1);
  const [stats, setStats] = useState({
    total_domains: 0,
    active_domains: 0,
    suspended_domains: 0,
    ssl_active_count: 0,
  });

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Filters
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState("all");
  const [typeFilter, setTypeFilter] = useState("all");
  const [nodeFilter, setNodeFilter] = useState("all");

  // Selection for bulk actions
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  // Dialogs
  const [isBulkAddOpen, setIsBulkAddOpen] = useState(false);
  const [deletingDomain, setDeletingDomain] = useState<AdminDomainItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const fetchDomains = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await listAdminDomains({
        page,
        perPage,
        search: searchQuery.trim() || undefined,
        status: statusFilter,
        type: typeFilter,
      });
      setDomains(res.items || []);
      setTotal(res.total || 0);
      setTotalPages(res.total_pages || Math.max(1, Math.ceil((res.total || 0) / perPage)));
      if (res.stats) {
        setStats(res.stats);
      }
    } catch (err: any) {
      setError(err?.message || t("adminDomains.errors.loadFailed", "Failed to load admin domains"));
    } finally {
      setIsLoading(false);
    }
  }, [page, perPage, searchQuery, statusFilter, typeFilter, nodeFilter, t]);

  useEffect(() => {
    fetchDomains();
  }, [fetchDomains]);

  // Bulk selection handlers
  const handleSelectAll = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.checked) {
      setSelectedIds(new Set(domains.map((d) => d.id)));
    } else {
      setSelectedIds(new Set());
    }
  };

  const handleSelectRow = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    setSelectedIds(next);
  };

  const handleToggleSuspend = async (domain: AdminDomainItem) => {
    const isSuspending = domain.status !== "suspended";
    const endpoint = `/api/v1/admin/domains/${domain.id}/${isSuspending ? "suspend" : "unsuspend"}`;

    try {
      await apiFetch(endpoint, { method: "POST" });
      toast({
        title: t("common.success", "Success"),
        description: isSuspending
          ? t("adminDomains.toast.suspended", "Domain '{{name}}' has been suspended", { name: domain.name })
          : t("adminDomains.toast.unsuspended", "Domain '{{name}}' has been unsuspended", { name: domain.name }),
      });
      fetchDomains();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("adminDomains.errors.statusFailed", "Failed to update domain status"),
        variant: "destructive",
      });
    }
  };

  const handleDeleteDomain = async () => {
    if (!deletingDomain) return;
    try {
      setIsDeleting(true);
      await apiFetch(`/api/v1/admin/domains/${deletingDomain.id}`, { method: "DELETE" });
      toast({
        title: t("common.success", "Success"),
        description: t("adminDomains.toast.deleted", "Domain '{{name}}' deleted successfully", {
          name: deletingDomain.name,
        }),
      });
      setDeletingDomain(null);
      fetchDomains();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("adminDomains.errors.deleteFailed", "Failed to delete domain"),
        variant: "destructive",
      });
    } finally {
      setIsDeleting(false);
    }
  };

  const handleBulkSuspend = async (suspend: boolean) => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;

    try {
      await bulkSuspend(ids, suspend);
      toast({
        title: t("common.success", "Success"),
        description: t("adminDomains.toast.bulkUpdated", "Updated {{count}} domains", { count: ids.length }),
      });
      setSelectedIds(new Set());
      fetchDomains();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || "Bulk operation failed",
        variant: "destructive",
      });
    }
  };

  const renderTypeBadge = (type: DomainType, isPrimary: boolean) => {
    if (isPrimary) {
      return (
        <Badge variant="default" className="bg-primary/90 text-primary-foreground font-semibold">
          {t("domains.types.primary", "Primary")}
        </Badge>
      );
    }
    switch (type) {
      case "addon":
        return <Badge variant="secondary">{t("domains.types.addon", "Addon")}</Badge>;
      case "subdomain":
        return <Badge variant="outline">{t("domains.types.subdomain", "Subdomain")}</Badge>;
      case "alias":
        return <Badge variant="outline" className="border-dashed">{t("domains.types.alias", "Alias")}</Badge>;
      default:
        return <Badge variant="outline">{type}</Badge>;
    }
  };

  const renderSslBadge = (status: SslStatus) => {
    switch (status) {
      case "active":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            <ShieldCheck className="h-4 w-4" />
            {t("domains.ssl.active", "Active")}
          </span>
        );
      case "pending":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            <Clock className="h-4 w-4 animate-spin" />
            {t("domains.ssl.pending", "Issuing")}
          </span>
        );
      case "expired":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-rose-600 dark:text-rose-400">
            <AlertCircle className="h-4 w-4" />
            {t("domains.ssl.expired", "Expired")}
          </span>
        );
      case "self_signed":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-orange-600 dark:text-orange-400">
            <ShieldAlert className="h-4 w-4" />
            {t("domains.ssl.selfSigned", "Self-Signed")}
          </span>
        );
      case "none":
      default:
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
            <ShieldOff className="h-4 w-4" />
            {t("domains.ssl.none", "No SSL")}
          </span>
        );
    }
  };

  const isAllSelected = domains.length > 0 && domains.every((d) => selectedIds.has(d.id));

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Globe className="h-6 w-6 text-primary" />
            {t("adminDomains.title", "All Domains")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "adminDomains.subtitle",
              "Manage virtual hosts, tenant owners, cluster nodes, and SSL certificates across all tenants."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchDomains}
            disabled={isLoading}
            className="h-9 gap-1.5"
            data-testid="btn-refresh-admin-domains"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            <span className="hidden sm:inline">{t("common.refresh", "Refresh")}</span>
          </Button>

          <Button
            size="sm"
            onClick={() => setIsBulkAddOpen(true)}
            className="h-9 gap-1.5"
            data-testid="btn-bulk-add-domains"
          >
            <Layers className="h-4 w-4" />
            {t("adminDomains.bulkAdd", "Bulk Add (Queue)")}
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4">
        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("adminDomains.metrics.total", "Total Domains")}
            </p>
            <p className="text-2xl font-bold mt-1">{stats.total_domains}</p>
          </div>
          <Globe className="h-8 w-8 text-primary/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("adminDomains.metrics.active", "Active")}
            </p>
            <p className="text-2xl font-bold text-emerald-600 dark:text-emerald-400 mt-1">
              {stats.active_domains}
            </p>
          </div>
          <CheckCircle2 className="h-8 w-8 text-emerald-500/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("adminDomains.metrics.suspended", "Suspended")}
            </p>
            <p className="text-2xl font-bold text-rose-600 dark:text-rose-400 mt-1">
              {stats.suspended_domains}
            </p>
          </div>
          <Ban className="h-8 w-8 text-rose-500/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("adminDomains.metrics.sslActive", "Active SSL")}
            </p>
            <p className="text-2xl font-bold text-primary mt-1">
              {stats.ssl_active_count}
            </p>
          </div>
          <ShieldCheck className="h-8 w-8 text-primary/40 shrink-0" />
        </Card>
      </div>

      {/* Filter Toolbar */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              setPage(1);
            }}
            placeholder={t("adminDomains.searchPlaceholder", "Search domain or tenant username...")}
            className="pl-9 h-9"
            data-testid="input-search-admin-domains"
          />
        </div>

        <select
          value={statusFilter}
          onChange={(e) => {
            setStatusFilter(e.target.value);
            setPage(1);
          }}
          data-testid="select-filter-status"
          className="h-9 px-3 rounded-md border border-input bg-background text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <option value="all">{t("adminDomains.filters.allStatuses", "All Statuses")}</option>
          <option value="active">{t("adminDomains.filters.active", "Active")}</option>
          <option value="suspended">{t("adminDomains.filters.suspended", "Suspended")}</option>
        </select>

        <select
          value={typeFilter}
          onChange={(e) => {
            setTypeFilter(e.target.value);
            setPage(1);
          }}
          data-testid="select-filter-type"
          className="h-9 px-3 rounded-md border border-input bg-background text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <option value="all">{t("adminDomains.filters.allTypes", "All Types")}</option>
          <option value="primary">{t("domains.types.primary", "Primary")}</option>
          <option value="addon">{t("domains.types.addon", "Addon")}</option>
          <option value="subdomain">{t("domains.types.subdomain", "Subdomain")}</option>
          <option value="alias">{t("domains.types.alias", "Alias")}</option>
        </select>

        <select
          value={nodeFilter}
          onChange={(e) => {
            setNodeFilter(e.target.value);
            setPage(1);
          }}
          data-testid="select-filter-node"
          className="h-9 px-3 rounded-md border border-input bg-background text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <option value="all">{t("adminDomains.filters.allNodes", "All Nodes")}</option>
          <option value="node-sgp-01">node-sgp-01</option>
          <option value="node-master-01">node-master-01</option>
          <option value="node-worker-02">node-worker-02</option>
        </select>
      </div>

      {/* Bulk Action Bar */}
      {selectedIds.size > 0 && (
        <div className="p-3 bg-primary/10 border border-primary/20 rounded-lg flex items-center justify-between text-xs sm:text-sm">
          <span className="font-medium text-foreground">
            {t("adminDomains.bulk.selectedCount", { count: selectedIds.size })}
          </span>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => handleBulkSuspend(true)}
              className="h-8 text-xs text-amber-700 dark:text-amber-300"
            >
              {t("adminDomains.bulk.suspendSelected", "Suspend Selected")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => handleBulkSuspend(false)}
              className="h-8 text-xs text-emerald-700 dark:text-emerald-300"
            >
              {t("adminDomains.bulk.unsuspendSelected", "Unsuspend Selected")}
            </Button>
          </div>
        </div>
      )}

      {/* Domains Table Card */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-medium">
            {t("adminDomains.tableTitle", "Cluster Web Domains")}
          </CardTitle>
          <CardDescription>
            {t(
              "adminDomains.tableSubtitle",
              "Live overview of host routing, owner mappings, and node distributions"
            )}
          </CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-10">
                  <input
                    type="checkbox"
                    checked={isAllSelected}
                    onChange={handleSelectAll}
                    aria-label="Select all"
                    className="rounded border-input text-primary focus:ring-primary h-4 w-4"
                  />
                </TableHead>
                <TableHead>{t("domains.table.domain", "Domain")}</TableHead>
                <TableHead>{t("domains.table.type", "Type")}</TableHead>
                <TableHead>{t("adminDomains.table.owner", "Owner (Tenant)")}</TableHead>
                <TableHead>{t("adminDomains.table.node", "Cluster Node")}</TableHead>
                <TableHead>{t("domains.table.ssl", "SSL Security")}</TableHead>
                <TableHead>{t("common.status", "Status")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={8} className="h-32 text-center text-muted-foreground">
                    <RefreshCw className="h-5 w-5 animate-spin mx-auto mb-2 text-primary" />
                    {t("common.loading", "Loading domains...")}
                  </TableCell>
                </TableRow>
              ) : error ? (
                <TableRow>
                  <TableCell colSpan={8} className="h-32 text-center text-destructive">
                    <AlertTriangle className="h-5 w-5 mx-auto mb-2" />
                    {error}
                  </TableCell>
                </TableRow>
              ) : domains.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8} className="h-32 text-center text-muted-foreground">
                    <Globe className="h-6 w-6 mx-auto mb-2 opacity-40" />
                    {searchQuery
                      ? t("domains.noResults", "No domains match your search query.")
                      : t("domains.noDomains", "No domains found.")}
                  </TableCell>
                </TableRow>
              ) : (
                domains.map((domain) => {
                  const isSelected = selectedIds.has(domain.id);
                  const isSuspended = domain.status === "suspended";

                  return (
                    <TableRow key={domain.id} data-testid={`admin-domain-row-${domain.id}`}>
                      {/* Checkbox */}
                      <TableCell>
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => handleSelectRow(domain.id)}
                          aria-label={`Select ${domain.name}`}
                          className="rounded border-input text-primary focus:ring-primary h-4 w-4"
                        />
                      </TableCell>

                      {/* Domain Name */}
                      <TableCell className="font-medium">
                        <div className="flex items-center gap-2">
                          <span className="font-mono font-semibold text-foreground">
                            {domain.name}
                          </span>
                          <a
                            href={`https://${domain.name}`}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-muted-foreground hover:text-foreground"
                            title={t("domains.openSite", "Open website in new tab")}
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                          </a>
                        </div>
                      </TableCell>

                      {/* Type Badge */}
                      <TableCell>{renderTypeBadge(domain.type, domain.is_primary)}</TableCell>

                      {/* Owner Tenant */}
                      <TableCell>
                        <div className="flex items-center gap-1.5 text-xs font-mono">
                          <User className="h-3.5 w-3.5 text-muted-foreground" />
                          <span className="font-semibold text-foreground">{domain.tenant_username}</span>
                        </div>
                      </TableCell>

                      {/* Cluster Node */}
                      <TableCell>
                        <div className="flex items-center gap-1 text-xs font-mono text-muted-foreground">
                          <Server className="h-3 w-3 shrink-0" />
                          <span className="truncate max-w-[130px]">{domain.node_name || domain.node_id}</span>
                        </div>
                      </TableCell>

                      {/* SSL Security */}
                      <TableCell>{renderSslBadge(domain.ssl_status)}</TableCell>

                      {/* Status */}
                      <TableCell>
                        {isSuspended ? (
                          <Badge variant="destructive" className="text-[11px] font-medium" data-testid={`domain-status-badge-${domain.id}`}>
                            {t("adminDomains.status.suspended", "Suspended")}
                          </Badge>
                        ) : (
                          <Badge variant="default" className="bg-emerald-600/90 text-white text-[11px] font-medium" data-testid={`domain-status-badge-${domain.id}`}>
                            {t("adminDomains.status.active", "Active")}
                          </Badge>
                        )}
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          {/* Suspend / Unsuspend Button */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className={`h-8 text-xs gap-1 ${
                              isSuspended
                                ? "text-emerald-600 hover:text-emerald-700 hover:bg-emerald-500/10"
                                : "text-amber-600 hover:text-amber-700 hover:bg-amber-500/10"
                            }`}
                            onClick={() => handleToggleSuspend(domain)}
                            data-testid={`btn-toggle-suspend-${domain.id}`}
                          >
                            {isSuspended ? (
                              <>
                                <CheckCircle2 className="h-3.5 w-3.5" />
                                <span className="hidden xl:inline">{t("adminDomains.actions.unsuspend", "Unsuspend")}</span>
                              </>
                            ) : (
                              <>
                                <Ban className="h-3.5 w-3.5" />
                                <span className="hidden xl:inline">{t("adminDomains.actions.suspend", "Suspend")}</span>
                              </>
                            )}
                          </Button>

                          {/* Delete Button */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-8 w-8 p-0 text-destructive hover:text-destructive hover:bg-destructive/10"
                            onClick={() => setDeletingDomain(domain)}
                            data-testid={`btn-delete-domain-${domain.id}`}
                            title={t("common.delete", "Delete domain")}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Pagination Footer */}
      {totalPages > 1 && (
        <div className="flex flex-col sm:flex-row items-center justify-between gap-4 text-xs text-muted-foreground pt-2">
          <div>
            {t("users.pagination.pageOf", { current: page, total: totalPages })}
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={page <= 1 || isLoading}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              data-testid="btn-prev-page"
            >
              {t("common.back", "Previous")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages || isLoading}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              data-testid="btn-next-page"
            >
              {t("common.next", "Next")}
            </Button>
          </div>
        </div>
      )}

      {/* Bulk Add Domains Dialog (Queue / Operations) */}
      <BulkAddDomainsDialog
        open={isBulkAddOpen}
        onOpenChange={setIsBulkAddOpen}
        onSuccess={() => {
          fetchDomains();
        }}
      />

      {/* Delete Confirmation Dialog */}
      <Dialog open={!!deletingDomain} onOpenChange={(open) => !open && setDeletingDomain(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              {t("domains.deleteDialog.title", "Delete Domain")}
            </DialogTitle>
          </DialogHeader>

          <div className="py-2 space-y-3 text-sm">
            <p>
              {t(
                "domains.deleteDialog.confirm",
                "Are you sure you want to remove domain {{name}}? Web server virtual host configurations will be removed.",
                { name: deletingDomain?.name }
              )}
            </p>
          </div>

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setDeletingDomain(null)}
              disabled={isDeleting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteDomain}
              disabled={isDeleting}
              data-testid="btn-confirm-delete-domain"
            >
              {isDeleting
                ? t("common.deleting", "Deleting...")
                : t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
