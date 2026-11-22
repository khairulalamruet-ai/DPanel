import React, { useState, useEffect, useMemo } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Button,
  Input,
  Badge,
  Card,
  CardContent,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { listUsers, bulkAction } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Users,
  Search,
  Filter,
  RefreshCw,
  UserPlus,
  ShieldAlert,
  ShieldCheck,
  Package,
  HardDrive,
  LogIn,
  MoreVertical,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  CheckCircle2,
  XCircle,
  AlertCircle,
  AlertTriangle,
  Trash2,
} from "lucide-react";
import { UserItem, UsersListResponse, UserStatus } from "./types";
import { ChangePlanDialog, AVAILABLE_PLANS } from "./ChangePlanDialog";
import { SuspendUserDialog } from "./SuspendUserDialog";
import { AccountsTabs } from "../accounts/AccountsTabs";

export const AVAILABLE_NODES = [
  { id: "node-sgp-01", name: "node-sgp-01 (Singapore)" },
  { id: "node-fra-01", name: "node-fra-01 (Frankfurt)" },
  { id: "node-nyc-01", name: "node-nyc-01 (New York)" },
];

export function formatBytes(bytes: number): string {
  if (bytes === -1) return "∞";
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function UsersPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  // Data state
  const [users, setUsers] = useState<UserItem[]>([]);
  const [total, setTotal] = useState(0);
  const [totalPages, setTotalPages] = useState(1);
  const [isLoading, setIsLoading] = useState(true);

  // Filters & Pagination state
  const [page, setPage] = useState(1);
  const [perPage, setPerPage] = useState(10);
  const [searchTerm, setSearchTerm] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [planFilter, setPlanFilter] = useState<string>("all");
  const [nodeFilter, setNodeFilter] = useState<string>("all");

  // Selection state
  const [selectedUserIds, setSelectedUserIds] = useState<Set<string>>(new Set());

  // Dialog state
  const [changePlanUsers, setChangePlanUsers] = useState<UserItem[] | null>(null);
  const [suspendDialogData, setSuspendDialogData] = useState<{
    users: UserItem[];
    action: "suspend" | "unsuspend";
  } | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const isFirstSearch = React.useRef(true);

  // Debounce search
  useEffect(() => {
    if (isFirstSearch.current) {
      isFirstSearch.current = false;
      return;
    }
    const timer = setTimeout(() => {
      setDebouncedSearch(searchTerm);
      setPage(1); // reset to page 1 on new search
    }, 300);
    return () => clearTimeout(timer);
  }, [searchTerm]);

  const fetchUsers = async () => {
    setIsLoading(true);
    try {
      const data = await listUsers({
        page,
        perPage,
        search: debouncedSearch.trim() || undefined,
        status: statusFilter,
        planId: planFilter,
        nodeId: nodeFilter,
      });

      setUsers(data.items || []);
      setTotal(data.total || 0);
      setTotalPages(data.total_pages || Math.max(1, Math.ceil((data.total || 0) / perPage)));
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("users.toast.loadError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchUsers();
    // Clear selection on page/filter change
    setSelectedUserIds(new Set());
  }, [page, perPage, debouncedSearch, statusFilter, planFilter, nodeFilter]);

  // Bulk selection helpers
  const isAllSelected = users.length > 0 && users.every((u) => selectedUserIds.has(u.id));
  const isSomeSelected = users.some((u) => selectedUserIds.has(u.id)) && !isAllSelected;

  const handleSelectAll = () => {
    if (isAllSelected) {
      setSelectedUserIds(new Set());
    } else {
      setSelectedUserIds(new Set(users.map((u) => u.id)));
    }
  };

  const handleToggleSelect = (userId: string) => {
    const next = new Set(selectedUserIds);
    if (next.has(userId)) {
      next.delete(userId);
    } else {
      next.add(userId);
    }
    setSelectedUserIds(next);
  };

  const selectedUsers = useMemo(
    () => users.filter((u) => selectedUserIds.has(u.id)),
    [users, selectedUserIds]
  );

  // Single user actions
  const handleSingleSuspendToggle = (user: UserItem) => {
    const action = user.status === "suspended" ? "unsuspend" : "suspend";
    setSuspendDialogData({
      users: [user],
      action,
    });
  };

  const handleSingleChangePlan = (user: UserItem) => {
    setChangePlanUsers([user]);
  };

  const handleImpersonate = async (user: UserItem) => {
    try {
      const res = await apiFetch<{ token: string; redirect_url: string }>(
        `/api/v1/admin/users/${user.id}/impersonate`,
        { method: "POST" }
      );
      toast({
        title: t("users.toast.impersonateSuccess", { username: user.username }),
        description: t("users.toast.impersonateDesc"),
      });
      // In real scenario opens client app or updates session
      if (res.redirect_url) {
        window.open(res.redirect_url, "_blank");
      }
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("users.toast.impersonateError"),
        variant: "destructive",
      });
    }
  };

  const handleDeleteUser = async (user: UserItem) => {
    if (!window.confirm(t("users.dialog.confirmDelete", { username: user.username }))) {
      return;
    }
    try {
      await apiFetch(`/api/v1/admin/users/${user.id}`, { method: "DELETE" });
      toast({
        title: t("common.appName"),
        description: t("users.toast.deleteSuccess", { username: user.username }),
      });
      fetchUsers();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("users.toast.deleteError"),
        variant: "destructive",
      });
    }
  };

  // Bulk actions handlers
  const handleBulkChangePlanConfirm = async (planId: string, planName: string) => {
    setIsSubmitting(true);
    try {
      const ids = changePlanUsers?.map((u) => u.id) || [];
      await bulkAction({ action: "change_plan", user_ids: ids, plan_id: planId });

      toast({
        title: t("common.appName"),
        description: t("users.toast.bulkPlanSuccess", {
          count: ids.length,
          plan: planName,
        }),
      });
      setSelectedUserIds(new Set());
      fetchUsers();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("users.toast.bulkPlanError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleSuspendConfirm = async (reason?: string) => {
    if (!suspendDialogData) return;
    setIsSubmitting(true);
    try {
      const { users: targetUsers, action } = suspendDialogData;
      const ids = targetUsers.map((u) => u.id);

      await bulkAction({ action, user_ids: ids, reason });

      toast({
        title: t("common.appName"),
        description:
          action === "suspend"
            ? t("users.toast.bulkSuspendSuccess", { count: ids.length })
            : t("users.toast.bulkUnsuspendSuccess", { count: ids.length }),
      });
      setSelectedUserIds(new Set());
      fetchUsers();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("users.toast.suspendError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleResetFilters = () => {
    setSearchTerm("");
    setDebouncedSearch("");
    setStatusFilter("all");
    setPlanFilter("all");
    setNodeFilter("all");
    setPage(1);
  };

  // Pagination navigation helper
  const startItem = total === 0 ? 0 : (page - 1) * perPage + 1;
  const endItem = Math.min(page * perPage, total);

  return (
    <div className="space-y-6">
      <AccountsTabs active="users" />
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Users className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("users.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("users.desc")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchUsers}
            disabled={isLoading}
            className="gap-2 text-xs"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("users.refresh")}
          </Button>
          <Button size="sm" className="gap-2" onClick={() => (window.location.href = "/users/new")}>
            <UserPlus className="h-4 w-4" />
            {t("users.createUser")}
          </Button>
        </div>
      </div>

      {/* Summary Stats Cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <Card className="p-4 flex items-center justify-between border bg-card">
          <div>
            <p className="text-xs font-medium text-muted-foreground">{t("users.stats.total")}</p>
            <p className="text-2xl font-bold text-foreground mt-1">{total.toLocaleString()}</p>
          </div>
          <div className="h-10 w-10 rounded-full bg-primary/10 flex items-center justify-center text-primary">
            <Users className="h-5 w-5" />
          </div>
        </Card>
        <Card className="p-4 flex items-center justify-between border bg-card">
          <div>
            <p className="text-xs font-medium text-muted-foreground">{t("users.stats.active")}</p>
            <p className="text-2xl font-bold text-emerald-600 dark:text-emerald-400 mt-1">
              {users.filter((u) => u.status === "active").length}
            </p>
          </div>
          <div className="h-10 w-10 rounded-full bg-emerald-500/10 flex items-center justify-center text-emerald-500">
            <CheckCircle2 className="h-5 w-5" />
          </div>
        </Card>
        <Card className="p-4 flex items-center justify-between border bg-card">
          <div>
            <p className="text-xs font-medium text-muted-foreground">{t("users.stats.suspended")}</p>
            <p className="text-2xl font-bold text-destructive mt-1">
              {users.filter((u) => u.status === "suspended").length}
            </p>
          </div>
          <div className="h-10 w-10 rounded-full bg-destructive/10 flex items-center justify-center text-destructive">
            <XCircle className="h-5 w-5" />
          </div>
        </Card>
        <Card className="p-4 flex items-center justify-between border bg-card">
          <div>
            <p className="text-xs font-medium text-muted-foreground">{t("users.stats.storageAllocated")}</p>
            <p className="text-2xl font-bold text-foreground mt-1">
              {formatBytes(
                users.reduce((acc, u) => acc + (u.disk_used_bytes > 0 ? u.disk_used_bytes : 0), 0)
              )}
            </p>
          </div>
          <div className="h-10 w-10 rounded-full bg-blue-500/10 flex items-center justify-center text-blue-500">
            <HardDrive className="h-5 w-5" />
          </div>
        </Card>
      </div>

      {/* Filter & Search Toolbar */}
      <div className="flex flex-col md:flex-row gap-3 items-stretch md:items-center justify-between bg-card p-3 rounded-lg border">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            placeholder={t("users.filter.searchPlaceholder")}
            className="pl-9 h-9 text-xs"
          />
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Status Filter */}
          <select
            value={statusFilter}
            onChange={(e) => {
              setStatusFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-input bg-background px-2.5 py-1 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allStatuses")}</option>
            <option value="active">{t("users.status.active")}</option>
            <option value="suspended">{t("users.status.suspended")}</option>
            <option value="pending">{t("users.status.pending")}</option>
          </select>

          {/* Plan Filter */}
          <select
            value={planFilter}
            onChange={(e) => {
              setPlanFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-input bg-background px-2.5 py-1 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allPlans")}</option>
            {AVAILABLE_PLANS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>

          {/* Node Filter */}
          <select
            value={nodeFilter}
            onChange={(e) => {
              setNodeFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-input bg-background px-2.5 py-1 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allNodes")}</option>
            {AVAILABLE_NODES.map((n) => (
              <option key={n.id} value={n.id}>
                {n.name}
              </option>
            ))}
          </select>

          {(searchTerm || statusFilter !== "all" || planFilter !== "all" || nodeFilter !== "all") && (
            <Button
              variant="ghost"
              size="sm"
              onClick={handleResetFilters}
              className="text-xs h-9 px-2 text-muted-foreground hover:text-foreground"
            >
              {t("users.filter.reset")}
            </Button>
          )}
        </div>
      </div>

      {/* Bulk Action Bar (when rows are selected) */}
      {selectedUserIds.size > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-3 p-3 bg-primary/10 border border-primary/20 rounded-lg animate-in fade-in-50">
          <div className="flex items-center gap-2">
            <Badge variant="default" className="font-mono">
              {selectedUserIds.size}
            </Badge>
            <span className="text-xs font-semibold text-foreground">
              {t("users.bulk.selectedCount", { count: selectedUserIds.size })}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setChangePlanUsers(selectedUsers)}
              className="gap-1.5 text-xs h-8"
            >
              <Package className="h-3.5 w-3.5" />
              {t("users.bulk.changePlan")}
            </Button>
            <Button
              size="sm"
              variant="destructive"
              onClick={() =>
                setSuspendDialogData({
                  users: selectedUsers,
                  action: "suspend",
                })
              }
              className="gap-1.5 text-xs h-8"
            >
              <ShieldAlert className="h-3.5 w-3.5" />
              {t("users.bulk.suspend")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                setSuspendDialogData({
                  users: selectedUsers,
                  action: "unsuspend",
                })
              }
              className="gap-1.5 text-xs h-8 text-emerald-600 dark:text-emerald-400 hover:text-emerald-700"
            >
              <ShieldCheck className="h-3.5 w-3.5" />
              {t("users.bulk.unsuspend")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setSelectedUserIds(new Set())}
              className="text-xs h-8 text-muted-foreground"
            >
              {t("users.bulk.clear")}
            </Button>
          </div>
        </div>
      )}

      {/* Users Table */}
      <Card className="border shadow-sm">
        <CardContent className="p-0">
          <div className="overflow-x-auto">
            <Table>
              <TableHeader className="bg-muted/50">
                <TableRow>
                  <TableHead className="w-10 px-4">
                    <input
                      type="checkbox"
                      aria-label={t("users.table.selectAll")}
                      checked={isAllSelected}
                      ref={(input) => {
                        if (input) input.indeterminate = isSomeSelected;
                      }}
                      onChange={handleSelectAll}
                      className="rounded border-input text-primary focus:ring-primary h-4 w-4 cursor-pointer"
                    />
                  </TableHead>
                  <TableHead className="w-[24%]">{t("users.table.user")}</TableHead>
                  <TableHead className="w-[18%]">{t("users.table.email")}</TableHead>
                  <TableHead className="w-[14%]">{t("users.table.plan")}</TableHead>
                  <TableHead className="w-[12%]">{t("users.table.node")}</TableHead>
                  <TableHead className="w-[10%]">{t("users.table.status")}</TableHead>
                  <TableHead className="w-[12%]">{t("users.table.diskUsage")}</TableHead>
                  <TableHead className="w-[10%] text-right pr-4">{t("users.table.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {isLoading ? (
                  <TableRow>
                    <TableCell colSpan={8} className="h-32 text-center text-sm text-muted-foreground">
                      <div className="flex items-center justify-center gap-2">
                        <RefreshCw className="h-4 w-4 animate-spin text-primary" />
                        <span>{t("common.loading")}</span>
                      </div>
                    </TableCell>
                  </TableRow>
                ) : users.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={8} className="h-32 text-center text-sm text-muted-foreground">
                      {t("users.table.noUsersFound")}
                    </TableCell>
                  </TableRow>
                ) : (
                  users.map((user) => {
                    const isSelected = selectedUserIds.has(user.id);
                    const diskUsedFormatted = formatBytes(user.disk_used_bytes);
                    const diskLimitFormatted = formatBytes(user.disk_limit_bytes);
                    const diskPercent =
                      user.disk_limit_bytes > 0
                        ? Math.min(100, Math.round((user.disk_used_bytes / user.disk_limit_bytes) * 100))
                        : 0;

                    return (
                      <TableRow
                        key={user.id}
                        data-state={isSelected ? "selected" : undefined}
                        className={isSelected ? "bg-muted/40" : undefined}
                      >
                        <TableCell className="px-4">
                          <input
                            type="checkbox"
                            aria-label={`Select ${user.username}`}
                            checked={isSelected}
                            onChange={() => handleToggleSelect(user.id)}
                            className="rounded border-input text-primary focus:ring-primary h-4 w-4 cursor-pointer"
                          />
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2.5">
                            <div className="h-8 w-8 rounded-full bg-primary/10 flex items-center justify-center font-bold text-xs text-primary uppercase shrink-0">
                              {user.username.slice(0, 2)}
                            </div>
                            <div className="min-w-0">
                              <p className="font-semibold text-foreground text-sm truncate">
                                {user.username}
                              </p>
                              {user.primary_domain && (
                                <p className="text-[11px] text-muted-foreground font-mono truncate">
                                  {user.primary_domain}
                                </p>
                              )}
                            </div>
                          </div>
                        </TableCell>
                        <TableCell>
                          <span className="text-xs text-foreground truncate block max-w-[180px]">
                            {user.email}
                          </span>
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline" className="font-normal text-xs">
                            {user.plan_name}
                          </Badge>
                        </TableCell>
                        <TableCell>
                          <div className="flex flex-col gap-1 items-start">
                            <span className="text-xs font-mono text-muted-foreground">
                              {user.node_name || user.node_id}
                            </span>
                            {(user.node_status === "offline" || user.is_node_offline) && (
                              <Badge
                                variant="destructive"
                                data-testid="tenant-node-offline-badge"
                                className="text-[10px] px-1.5 py-0 font-medium flex items-center gap-1"
                              >
                                <AlertTriangle className="h-2.5 w-2.5" />
                                {t("users.nodeOffline")}
                              </Badge>
                            )}
                          </div>
                        </TableCell>
                        <TableCell>
                          {user.status === "active" ? (
                            <Badge variant="default" className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30 text-xs">
                              {t("users.status.active")}
                            </Badge>
                          ) : user.status === "suspended" ? (
                            <Badge variant="destructive" className="text-xs">
                              {t("users.status.suspended")}
                            </Badge>
                          ) : (
                            <Badge variant="secondary" className="text-xs">
                              {t("users.status.pending")}
                            </Badge>
                          )}
                        </TableCell>
                        <TableCell>
                          <div className="space-y-1">
                            <div className="flex items-center justify-between text-[11px] font-mono text-muted-foreground">
                              <span>{diskUsedFormatted}</span>
                              <span>/ {diskLimitFormatted}</span>
                            </div>
                            {user.disk_limit_bytes > 0 && (
                              <div className="w-full h-1.5 rounded-full bg-muted overflow-hidden">
                                <div
                                  className={`h-full rounded-full transition-all ${
                                    diskPercent > 90
                                      ? "bg-destructive"
                                      : diskPercent > 75
                                      ? "bg-amber-500"
                                      : "bg-primary"
                                  }`}
                                  style={{ width: `${diskPercent}%` }}
                                />
                              </div>
                            )}
                          </div>
                        </TableCell>
                        <TableCell className="text-right pr-4">
                          <div className="flex items-center justify-end gap-1">
                            {/* Impersonate */}
                            <Button
                              variant="ghost"
                              size="sm"
                              title={t("users.actions.impersonate")}
                              aria-label={t("users.actions.impersonate")}
                              onClick={() => handleImpersonate(user)}
                              className="h-8 w-8 p-0 text-muted-foreground hover:text-primary"
                            >
                              <LogIn className="h-4 w-4" />
                            </Button>

                            {/* Change Plan */}
                            <Button
                              variant="ghost"
                              size="sm"
                              title={t("users.actions.changePlan")}
                              aria-label={t("users.actions.changePlan")}
                              onClick={() => handleSingleChangePlan(user)}
                              className="h-8 w-8 p-0 text-muted-foreground hover:text-foreground"
                            >
                              <Package className="h-4 w-4" />
                            </Button>

                            {/* Suspend / Unsuspend */}
                            <Button
                              variant="ghost"
                              size="sm"
                              title={
                                user.status === "suspended"
                                  ? t("users.actions.unsuspend")
                                  : t("users.actions.suspend")
                              }
                              aria-label={
                                user.status === "suspended"
                                  ? t("users.actions.unsuspend")
                                  : t("users.actions.suspend")
                              }
                              onClick={() => handleSingleSuspendToggle(user)}
                              className={`h-8 w-8 p-0 ${
                                user.status === "suspended"
                                  ? "text-emerald-600 hover:text-emerald-700"
                                  : "text-muted-foreground hover:text-destructive"
                              }`}
                            >
                              {user.status === "suspended" ? (
                                <ShieldCheck className="h-4 w-4" />
                              ) : (
                                <ShieldAlert className="h-4 w-4" />
                              )}
                            </Button>

                            {/* Delete */}
                            <Button
                              variant="ghost"
                              size="sm"
                              title={t("users.actions.delete")}
                              aria-label={t("users.actions.delete")}
                              onClick={() => handleDeleteUser(user)}
                              className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
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
          </div>

          {/* Pagination Footer */}
          <div className="flex flex-col sm:flex-row items-center justify-between gap-4 p-4 border-t bg-muted/20">
            <div className="flex items-center gap-3">
              <span className="text-xs text-muted-foreground">
                {t("users.pagination.showing", {
                  start: startItem,
                  end: endItem,
                  total: total.toLocaleString(),
                })}
              </span>

              {/* Page size selector */}
              <div className="flex items-center gap-1.5">
                <span className="text-xs text-muted-foreground">{t("users.pagination.perPage")}:</span>
                <select
                  value={perPage}
                  onChange={(e) => {
                    setPerPage(Number(e.target.value));
                    setPage(1);
                  }}
                  className="h-7 rounded border border-input bg-background px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  <option value={10}>10</option>
                  <option value={25}>25</option>
                  <option value={50}>50</option>
                  <option value={100}>100</option>
                </select>
              </div>
            </div>

            {/* Page navigation */}
            <div className="flex items-center gap-1">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1 || isLoading}
                onClick={() => setPage(1)}
                className="h-8 w-8 p-0"
                aria-label="First page"
              >
                <ChevronsLeft className="h-4 w-4" />
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1 || isLoading}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                className="h-8 w-8 p-0"
                aria-label="Previous page"
              >
                <ChevronLeft className="h-4 w-4" />
              </Button>

              <span className="px-3 text-xs font-medium text-foreground">
                {t("users.pagination.pageOf", { current: page, total: totalPages })}
              </span>

              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={page >= totalPages || isLoading}
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                className="h-8 w-8 p-0"
                aria-label="Next page"
              >
                <ChevronRight className="h-4 w-4" />
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages || isLoading}
                onClick={() => setPage(totalPages)}
                className="h-8 w-8 p-0"
                aria-label="Last page"
              >
                <ChevronsRight className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Change Plan Dialog */}
      {changePlanUsers && (
        <ChangePlanDialog
          open={Boolean(changePlanUsers)}
          onOpenChange={(open) => {
            if (!open) setChangePlanUsers(null);
          }}
          users={changePlanUsers}
          onConfirm={handleBulkChangePlanConfirm}
          isSubmitting={isSubmitting}
        />
      )}

      {/* Suspend / Unsuspend Dialog */}
      {suspendDialogData && (
        <SuspendUserDialog
          open={Boolean(suspendDialogData)}
          onOpenChange={(open) => {
            if (!open) setSuspendDialogData(null);
          }}
          users={suspendDialogData.users}
          action={suspendDialogData.action}
          onConfirm={handleSuspendConfirm}
          isSubmitting={isSubmitting}
        />
      )}
    </div>
  );
}
