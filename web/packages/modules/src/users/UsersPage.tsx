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
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { listUsers, bulkAction } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Search,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Package,
  LogIn,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  AlertTriangle,
  Trash2,
  ChevronDown,
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

type ColumnKey = "email" | "status" | "package" | "node" | "memory" | "cpu" | "disk" | "inodes" | "created" | "login" | "actions";

const COLUMNS: { key: ColumnKey; labelKey: string; fallback: string }[] = [
  { key: "email", labelKey: "users.table.email", fallback: "Email" },
  { key: "status", labelKey: "users.table.status", fallback: "Status" },
  { key: "package", labelKey: "users.table.package", fallback: "Package" },
  { key: "node", labelKey: "users.table.node", fallback: "Node" },
  { key: "memory", labelKey: "users.table.memory", fallback: "Memory %" },
  { key: "cpu", labelKey: "users.table.cpu", fallback: "CPU %" },
  { key: "disk", labelKey: "users.table.disk", fallback: "Disk %" },
  { key: "inodes", labelKey: "users.table.inodes", fallback: "Inodes %" },
  { key: "created", labelKey: "users.table.created", fallback: "Created" },
  { key: "login", labelKey: "users.table.login", fallback: "Log in" },
  { key: "actions", labelKey: "users.table.actions", fallback: "Actions" },
];

const DEFAULT_COLUMNS: Record<ColumnKey, boolean> = {
  email: false,
  status: true,
  package: true,
  node: false,
  memory: true,
  cpu: true,
  disk: true,
  inodes: true,
  created: true,
  login: true,
  actions: true,
};

/** "3mo ago", "2h ago": how long ago a time was, in the page's language. */
export function timeAgo(iso: string, now: number = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  const lang = typeof document !== "undefined" && document.documentElement.lang ? document.documentElement.lang : "en";
  const rtf = new Intl.RelativeTimeFormat(lang, { numeric: "always", style: "narrow" });
  for (const [unit, size] of units) {
    if (seconds >= size) return rtf.format(-Math.floor(seconds / size), unit);
  }
  return rtf.format(-seconds, "second");
}

/** A small ring that shows how much of a plan limit is used; empty when it is not known. */
export function UsageRing({ percent, label }: { percent?: number | null; label: string }) {
  const known = typeof percent === "number";
  const shown = known ? percent : 0;
  const radius = 13;
  const circumference = 2 * Math.PI * radius;
  const filled = (Math.min(100, Math.max(0, shown)) / 100) * circumference;
  const color = !known ? "stroke-transparent" : shown > 90 ? "stroke-destructive" : shown > 75 ? "stroke-amber-500" : "stroke-primary";
  return (
    <div
      className="relative h-8 w-8"
      role="img"
      aria-label={known ? `${label}: ${shown}%` : `${label}: –`}
      data-testid="usage-ring"
    >
      <svg viewBox="0 0 32 32" className="h-8 w-8 -rotate-90">
        <circle cx="16" cy="16" r={radius} fill="none" strokeWidth="3" className="stroke-muted" />
        <circle
          cx="16"
          cy="16"
          r={radius}
          fill="none"
          strokeWidth="3"
          strokeLinecap="round"
          strokeDasharray={`${filled} ${circumference}`}
          className={color}
        />
      </svg>
      <span className="absolute inset-0 flex items-center justify-center text-[10px] font-medium text-foreground">
        {known ? shown : "–"}
      </span>
    </div>
  );
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
  const [columns, setColumns] = useState<Record<ColumnKey, boolean>>(DEFAULT_COLUMNS);
  const [columnsOpen, setColumnsOpen] = useState(false);

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
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{t("users.accountsTitle", "User accounts")}</h1>
          <p className="text-sm text-muted-foreground mt-1" data-testid="users-count">
            {t("users.accountsCount", { count: total, defaultValue: "{{count}} user accounts on this server." })}
          </p>
        </div>
        <Button size="sm" onClick={() => (window.location.href = "/users/new")}>
          {t("users.createNew", "Create new")}
        </Button>
      </div>

      {/* Filter & Search Toolbar */}
      <div className="flex flex-col md:flex-row gap-3 items-stretch md:items-center justify-between">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative w-full sm:w-56">
            <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              placeholder={t("users.filter.searchPlaceholder")}
              className="pl-9 h-9 text-xs"
            />
          </div>
          <select
            aria-label={t("users.filter.status", "Status")}
            value={statusFilter}
            onChange={(e) => {
              setStatusFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-dashed border-input bg-background px-2.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allStatuses")}</option>
            <option value="active">{t("users.status.active")}</option>
            <option value="suspended">{t("users.status.suspended")}</option>
            <option value="pending">{t("users.status.pending")}</option>
          </select>
          <select
            aria-label={t("users.filter.package", "Package")}
            value={planFilter}
            onChange={(e) => {
              setPlanFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-dashed border-input bg-background px-2.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allPlans")}</option>
            {AVAILABLE_PLANS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <select
            aria-label={t("users.filter.node", "Node")}
            value={nodeFilter}
            onChange={(e) => {
              setNodeFilter(e.target.value);
              setPage(1);
            }}
            className="h-9 rounded-md border border-dashed border-input bg-background px-2.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="all">{t("users.filter.allNodes")}</option>
            {AVAILABLE_NODES.map((n) => (
              <option key={n.id} value={n.id}>
                {n.name}
              </option>
            ))}
          </select>
          {(searchTerm || statusFilter !== "all" || planFilter !== "all" || nodeFilter !== "all") && (
            <Button variant="ghost" size="sm" onClick={handleResetFilters} className="text-xs h-9 px-2 text-muted-foreground hover:text-foreground">
              {t("users.filter.reset")}
            </Button>
          )}
        </div>

        {/* Show columns */}
        <div className="relative">
          <Button
            variant="outline"
            size="sm"
            aria-haspopup="true"
            aria-expanded={columnsOpen}
            onClick={() => setColumnsOpen((o) => !o)}
            className="h-9 gap-2 text-xs"
          >
            {t("users.showColumns", "Show columns")}
            <ChevronDown className="h-4 w-4" />
          </Button>
          {columnsOpen && (
            <div className="absolute right-0 z-20 mt-1 w-48 rounded-md border bg-popover p-2 shadow-md" role="menu">
              {COLUMNS.map((c) => (
                <label key={c.key} className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-muted">
                  <input
                    type="checkbox"
                    checked={columns[c.key]}
                    onChange={() => setColumns((prev) => ({ ...prev, [c.key]: !prev[c.key] }))}
                    className="h-3.5 w-3.5"
                  />
                  {t(c.labelKey, c.fallback)}
                </label>
              ))}
            </div>
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
      <div className="rounded-lg border">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
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
                <TableHead>{t("users.table.username", "Username")}</TableHead>
                {columns.email && <TableHead>{t("users.table.email")}</TableHead>}
                {columns.status && <TableHead>{t("users.table.status")}</TableHead>}
                {columns.package && <TableHead>{t("users.table.package", "Package")}</TableHead>}
                {columns.node && <TableHead>{t("users.table.node")}</TableHead>}
                {columns.memory && <TableHead>{t("users.table.memory", "Memory %")}</TableHead>}
                {columns.cpu && <TableHead>{t("users.table.cpu", "CPU %")}</TableHead>}
                {columns.disk && <TableHead>{t("users.table.disk", "Disk %")}</TableHead>}
                {columns.inodes && <TableHead>{t("users.table.inodes", "Inodes %")}</TableHead>}
                {columns.created && <TableHead>{t("users.table.created", "Created")}</TableHead>}
                {columns.login && <TableHead className="text-center">{t("users.table.login", "Log in")}</TableHead>}
                {columns.actions && <TableHead className="text-right pr-4">{t("users.table.actions")}</TableHead>}
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={13} className="h-32 text-center text-sm text-muted-foreground">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="h-4 w-4 animate-spin text-primary" />
                      <span>{t("common.loading")}</span>
                    </div>
                  </TableCell>
                </TableRow>
              ) : users.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={13} className="h-32 text-center text-sm text-muted-foreground">
                    {t("users.table.noUsersFound")}
                  </TableCell>
                </TableRow>
              ) : (
                users.map((user) => {
                  const isSelected = selectedUserIds.has(user.id);
                  const suspended = user.status === "suspended";
                  return (
                    <TableRow
                      key={user.id}
                      data-state={isSelected ? "selected" : undefined}
                      className={`${isSelected ? "bg-muted/40" : ""} ${suspended ? "border-l-2 border-l-destructive" : ""}`}
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
                        <p className={`font-medium text-sm truncate ${suspended ? "text-destructive" : "text-foreground"}`}>
                          {user.username}
                        </p>
                        {user.primary_domain && (
                          <p className="text-[11px] text-muted-foreground font-mono truncate">{user.primary_domain}</p>
                        )}
                        {(user.node_status === "offline" || user.is_node_offline) && (
                          <Badge
                            variant="destructive"
                            data-testid="tenant-node-offline-badge"
                            className="mt-1 text-[10px] px-1.5 py-0 font-medium inline-flex items-center gap-1"
                          >
                            <AlertTriangle className="h-2.5 w-2.5" />
                            {t("users.nodeOffline")}
                          </Badge>
                        )}
                      </TableCell>
                      {columns.email && (
                        <TableCell>
                          <span className="text-xs text-foreground truncate block max-w-[180px]">{user.email}</span>
                        </TableCell>
                      )}
                      {columns.status && (
                        <TableCell>
                          {user.status === "active" ? (
                            <Badge variant="default" className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30 text-xs">
                              {t("users.status.active")}
                            </Badge>
                          ) : suspended ? (
                            <Badge variant="destructive" className="text-xs">
                              {t("users.status.suspended")}
                            </Badge>
                          ) : (
                            <Badge variant="secondary" className="text-xs">
                              {t("users.status.pending")}
                            </Badge>
                          )}
                        </TableCell>
                      )}
                      {columns.package && (
                        <TableCell className="text-sm text-foreground">{user.plan_name}</TableCell>
                      )}
                      {columns.node && (
                        <TableCell>
                          <span className="text-xs font-mono text-muted-foreground">{user.node_name || user.node_id}</span>
                        </TableCell>
                      )}
                      {columns.memory && (
                        <TableCell>
                          <UsageRing percent={user.usage?.memory_percent} label={t("users.table.memory", "Memory %")} />
                        </TableCell>
                      )}
                      {columns.cpu && (
                        <TableCell>
                          <UsageRing percent={user.usage?.cpu_percent} label={t("users.table.cpu", "CPU %")} />
                        </TableCell>
                      )}
                      {columns.disk && (
                        <TableCell>
                          <UsageRing percent={user.usage?.disk_percent} label={t("users.table.disk", "Disk %")} />
                        </TableCell>
                      )}
                      {columns.inodes && (
                        <TableCell>
                          <UsageRing percent={user.usage?.inodes_percent} label={t("users.table.inodes", "Inodes %")} />
                        </TableCell>
                      )}
                      {columns.created && (
                        <TableCell className="text-sm text-muted-foreground whitespace-nowrap" title={new Date(user.created_at).toLocaleString()}>
                          {timeAgo(user.created_at)}
                        </TableCell>
                      )}
                      {columns.login && (
                        <TableCell className="text-center">
                          {!suspended && (
                            <Button variant="outline" size="sm" className="gap-2 text-xs" onClick={() => handleImpersonate(user)}>
                              <LogIn className="h-3.5 w-3.5" />
                              {t("users.loginAsUser", "Log in as user")}
                            </Button>
                          )}
                        </TableCell>
                      )}
                      {columns.actions && (
                        <TableCell className="text-right pr-4">
                          <div className="flex items-center justify-end gap-1">
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
                            <Button
                              variant="ghost"
                              size="sm"
                              title={suspended ? t("users.actions.unsuspend") : t("users.actions.suspend")}
                              aria-label={suspended ? t("users.actions.unsuspend") : t("users.actions.suspend")}
                              onClick={() => handleSingleSuspendToggle(user)}
                              className={`h-8 w-8 p-0 ${suspended ? "text-emerald-600 hover:text-emerald-700" : "text-muted-foreground hover:text-destructive"}`}
                            >
                              {suspended ? <ShieldCheck className="h-4 w-4" /> : <ShieldAlert className="h-4 w-4" />}
                            </Button>
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
                      )}
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
      </div>

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
