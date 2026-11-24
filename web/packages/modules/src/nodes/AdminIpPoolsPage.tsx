import React, { useState, useEffect, useMemo } from "react";
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
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Globe,
  Plus,
  RefreshCw,
  Server,
  UserCheck,
  UserMinus,
  Trash2,
  Layers,
  Shield,
  Search,
  Filter,
} from "lucide-react";
import {
  NodeIpAddress,
  NodeIpPoolMetrics,
  NodeItem,
  AddNodeIpPayload,
  AllocateIpPayload,
  IpAddressType,
} from "./types";
import { AddIpDialog } from "./AddIpDialog";
import { AllocateIpDialog } from "./AllocateIpDialog";

export function AdminIpPoolsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [ips, setIps] = useState<NodeIpAddress[]>([]);
  const [nodes, setNodes] = useState<NodeItem[]>([]);
  const [metrics, setMetrics] = useState<NodeIpPoolMetrics | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Filters
  const [selectedNodeId, setSelectedNodeId] = useState<string>("ALL");
  const [selectedType, setSelectedType] = useState<string>("ALL");
  const [searchQuery, setSearchQuery] = useState<string>("");

  // Dialogs
  const [isAddOpen, setIsAddOpen] = useState(false);
  const [allocatingIp, setAllocatingIp] = useState<NodeIpAddress | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [actionIpId, setActionIpId] = useState<string | null>(null);

  const fetchPoolData = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const [ipsRes, nodesRes] = await Promise.all([
        apiFetch<{ items: NodeIpAddress[]; metrics: NodeIpPoolMetrics }>(
          "/api/v1/admin/cluster/ips"
        ),
        apiFetch<{ items: NodeItem[] }>("/api/v1/admin/nodes"),
      ]);
      setIps(ipsRes.items || []);
      setMetrics(ipsRes.metrics || null);
      setNodes(nodesRes.items || []);
    } catch (err: any) {
      setError(err?.message || "Failed to load IP pool data");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchPoolData();
  }, []);

  const handleAddIp = async (payload: AddNodeIpPayload) => {
    setIsSaving(true);
    try {
      const created = await apiFetch<NodeIpAddress>(
        `/api/v1/admin/cluster/nodes/${payload.node_id}/ips`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }
      );
      toast({
        title: t("common.appName"),
        description: t("nodes.ipPools.toast.addSuccess", { ip: created.ip_address }),
      });
      setIps((prev) => [created, ...prev]);
      fetchPoolData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to add IP address",
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  const handleAllocateIp = async (ipId: string, payload: AllocateIpPayload) => {
    setIsSaving(true);
    try {
      const target = ips.find((i) => i.id === ipId);
      if (!target) return;

      const updated = await apiFetch<NodeIpAddress>(
        `/api/v1/admin/cluster/nodes/${target.node_id}/ips/${ipId}/assign`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }
      );

      toast({
        title: t("common.appName"),
        description: t("nodes.ipPools.toast.allocatedSuccess", {
          ip: updated.ip_address,
          user: payload.tenant_username,
        }),
      });

      setIps((prev) => prev.map((item) => (item.id === ipId ? updated : item)));
      fetchPoolData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to allocate IP",
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  const handleReleaseIp = async (ip: NodeIpAddress) => {
    if (!window.confirm(t("nodes.ipPools.dialog.releaseConfirm", { ip: ip.ip_address }))) {
      return;
    }

    try {
      setActionIpId(ip.id);
      const updated = await apiFetch<NodeIpAddress>(
        `/api/v1/admin/cluster/nodes/${ip.node_id}/ips/${ip.id}/release`,
        {
          method: "POST",
        }
      );

      toast({
        title: t("common.appName"),
        description: t("nodes.ipPools.toast.releasedSuccess", { ip: ip.ip_address }),
      });

      setIps((prev) => prev.map((item) => (item.id === ip.id ? updated : item)));
      fetchPoolData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to release IP",
        variant: "destructive",
      });
    } finally {
      setActionIpId(null);
    }
  };

  const handleDeleteIp = async (ip: NodeIpAddress) => {
    if (ip.is_primary) {
      toast({
        title: t("common.error"),
        description: t("nodes.ipPools.toast.cannotDeletePrimary"),
        variant: "destructive",
      });
      return;
    }

    if (!window.confirm(t("nodes.ipPools.dialog.deleteConfirm", { ip: ip.ip_address }))) {
      return;
    }

    try {
      setActionIpId(ip.id);
      await apiFetch(`/api/v1/admin/cluster/nodes/${ip.node_id}/ips/${ip.id}`, {
        method: "DELETE",
      });

      toast({
        title: t("common.appName"),
        description: t("nodes.ipPools.toast.deleteSuccess", { ip: ip.ip_address }),
      });

      setIps((prev) => prev.filter((item) => item.id !== ip.id));
      fetchPoolData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to delete IP",
        variant: "destructive",
      });
    } finally {
      setActionIpId(null);
    }
  };

  // Filtered List
  const filteredIps = useMemo(() => {
    return ips.filter((ip) => {
      if (selectedNodeId !== "ALL" && ip.node_id !== selectedNodeId) {
        return false;
      }
      if (selectedType !== "ALL" && ip.type !== selectedType) {
        return false;
      }
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        const matchesIp = ip.ip_address.toLowerCase().includes(q);
        const matchesNode = ip.node_name.toLowerCase().includes(q);
        const matchesUser = (ip.assigned_tenant_username || "").toLowerCase().includes(q);
        const matchesDomain = (ip.assigned_domain || "").toLowerCase().includes(q);
        if (!matchesIp && !matchesNode && !matchesUser && !matchesDomain) {
          return false;
        }
      }
      return true;
    });
  }, [ips, selectedNodeId, selectedType, searchQuery]);

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Globe className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">
              {t("nodes.ipPools.title")}
            </h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">
            {t("nodes.ipPools.desc")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchPoolData}
            disabled={isLoading}
            className="gap-2"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.reload")}
          </Button>
          <Button
            size="sm"
            onClick={() => setIsAddOpen(true)}
            data-testid="btn-add-ip"
            className="gap-2"
          >
            <Plus className="h-4 w-4" />
            {t("nodes.ipPools.addIpBtn")}
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("nodes.ipPools.metrics.totalIps")}
            </span>
            <Globe className="h-4 w-4 text-primary" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold" data-testid="metric-total-ips">
              {metrics ? metrics.total_ips : ips.length}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("nodes.ipPools.metrics.acrossNodes", { count: nodes.length })}
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("nodes.ipPools.metrics.sharedIps")}
            </span>
            <Layers className="h-4 w-4 text-blue-500" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold">
              {metrics ? metrics.shared_ips : ips.filter((i) => i.type === "shared").length}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("nodes.ipPools.metrics.sharedPools")}
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("nodes.ipPools.metrics.dedicatedAssigned")}
            </span>
            <UserCheck className="h-4 w-4 text-emerald-500" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold" data-testid="metric-dedicated-assigned">
              {metrics
                ? metrics.dedicated_assigned
                : ips.filter((i) => i.type === "dedicated" && i.assigned_tenant_id).length}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("nodes.ipPools.metrics.allocatedTenants")}
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("nodes.ipPools.metrics.dedicatedAvailable")}
            </span>
            <Shield className="h-4 w-4 text-amber-500" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold" data-testid="metric-dedicated-available">
              {metrics
                ? metrics.dedicated_available
                : ips.filter((i) => i.type === "dedicated" && !i.assigned_tenant_id).length}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("nodes.ipPools.metrics.readyToAssign")}
            </span>
          </div>
        </Card>
      </div>

      {/* Filter Toolbar */}
      <Card>
        <CardContent className="p-3">
          <div className="flex flex-col sm:flex-row gap-3 items-center justify-between">
            <div className="flex flex-wrap items-center gap-2.5 w-full sm:w-auto">
              {/* Node filter */}
              <div className="flex items-center gap-1.5">
                <Server className="h-3.5 w-3.5 text-muted-foreground" />
                <select
                  data-testid="filter-node-select"
                  value={selectedNodeId}
                  onChange={(e) => setSelectedNodeId(e.target.value)}
                  className="h-8 rounded-md border border-input bg-background px-2.5 text-xs text-foreground shadow-sm"
                >
                  <option value="ALL">{t("nodes.ipPools.filters.allNodes")}</option>
                  {nodes.map((n) => (
                    <option key={n.id} value={n.id}>
                      {n.name}
                    </option>
                  ))}
                </select>
              </div>

              {/* Type filter */}
              <div className="flex items-center gap-1.5">
                <Filter className="h-3.5 w-3.5 text-muted-foreground" />
                <select
                  data-testid="filter-type-select"
                  value={selectedType}
                  onChange={(e) => setSelectedType(e.target.value)}
                  className="h-8 rounded-md border border-input bg-background px-2.5 text-xs text-foreground shadow-sm"
                >
                  <option value="ALL">{t("nodes.ipPools.filters.allTypes")}</option>
                  <option value="shared">{t("nodes.ipPools.type.shared")}</option>
                  <option value="dedicated">{t("nodes.ipPools.type.dedicated")}</option>
                </select>
              </div>
            </div>

            {/* Search */}
            <div className="w-full sm:w-64">
              <Input
                data-testid="input-ip-search"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t("nodes.ipPools.filters.searchPlaceholder")}
                className="h-8 text-xs"
              />
            </div>
          </div>
        </CardContent>
      </Card>

      {/* IP Table */}
      <Card>
        <CardHeader className="pb-3 border-b">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base font-semibold">
                {t("nodes.ipPools.table.title")}
              </CardTitle>
              <CardDescription className="text-xs mt-0.5">
                {t("nodes.ipPools.table.desc")}
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="p-0">
          {isLoading ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
              {t("common.loading")}
            </div>
          ) : error ? (
            <div className="py-12 text-center space-y-3">
              <p className="text-sm text-destructive">{error}</p>
              <Button variant="outline" size="sm" onClick={fetchPoolData}>
                {t("common.retry")}
              </Button>
            </div>
          ) : filteredIps.length === 0 ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              {t("nodes.ipPools.table.empty")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[24%]">{t("nodes.ipPools.table.ipAddress")}</TableHead>
                    <TableHead className="w-[18%]">{t("nodes.ipPools.table.node")}</TableHead>
                    <TableHead className="w-[14%]">{t("nodes.ipPools.table.type")}</TableHead>
                    <TableHead className="w-[14%]">{t("nodes.ipPools.table.status")}</TableHead>
                    <TableHead className="w-[20%]">{t("nodes.ipPools.table.assignment")}</TableHead>
                    <TableHead className="w-[10%] text-right">
                      {t("nodes.ipPools.table.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredIps.map((ip) => (
                    <TableRow
                      key={ip.id}
                      data-testid={`ip-row-${ip.id}`}
                      className="hover:bg-muted/50 transition-colors"
                    >
                      {/* IP & Netmask */}
                      <TableCell>
                        <div className="flex flex-col">
                          <div className="flex items-center gap-1.5">
                            <span className="font-mono text-xs font-bold text-foreground">
                              {ip.ip_address}
                            </span>
                            {ip.is_primary && (
                              <Badge
                                variant="secondary"
                                className="text-[10px] px-1 py-0 bg-blue-500/10 text-blue-600 border border-blue-500/20"
                              >
                                {t("nodes.ipPools.badge.primary")}
                              </Badge>
                            )}
                          </div>
                          <span className="font-mono text-[11px] text-muted-foreground">
                            {ip.subnet_mask} ({ip.interface_name})
                          </span>
                        </div>
                      </TableCell>

                      {/* Node Name */}
                      <TableCell>
                        <div className="flex items-center gap-1.5 text-xs">
                          <Server className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                          <span className="font-medium text-foreground">{ip.node_name}</span>
                        </div>
                      </TableCell>

                      {/* IP Type */}
                      <TableCell>
                        {ip.type === "shared" ? (
                          <Badge
                            variant="secondary"
                            data-testid="badge-ip-type-shared"
                            className="text-[10px] font-medium bg-muted text-muted-foreground"
                          >
                            <Layers className="h-2.5 w-2.5 mr-1" />
                            {t("nodes.ipPools.type.shared")}
                          </Badge>
                        ) : (
                          <Badge
                            variant="outline"
                            data-testid="badge-ip-type-dedicated"
                            className="text-[10px] font-medium bg-amber-500/10 text-amber-600 border-amber-500/20"
                          >
                            <Shield className="h-2.5 w-2.5 mr-1" />
                            {t("nodes.ipPools.type.dedicated")}
                          </Badge>
                        )}
                      </TableCell>

                      {/* Status */}
                      <TableCell>
                        {ip.status === "active" ? (
                          <Badge
                            variant="secondary"
                            className="text-[10px] bg-emerald-500/10 text-emerald-600 border border-emerald-500/20 font-medium"
                          >
                            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 mr-1.5" />
                            {t("nodes.ipPools.status.active")}
                          </Badge>
                        ) : (
                          <Badge
                            variant="outline"
                            className="text-[10px] bg-muted/40 text-muted-foreground font-medium"
                          >
                            <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground mr-1.5" />
                            {t("nodes.ipPools.status.available")}
                          </Badge>
                        )}
                      </TableCell>

                      {/* Assignment */}
                      <TableCell>
                        {ip.type === "shared" ? (
                          <span className="text-xs text-muted-foreground">
                            {t("nodes.ipPools.assignment.sharedPool", {
                              count: ip.shared_tenants_count ?? 12,
                            })}
                          </span>
                        ) : ip.assigned_tenant_username ? (
                          <div className="flex flex-col text-xs">
                            <span
                              className="font-semibold text-foreground"
                              data-testid={`assigned-tenant-${ip.id}`}
                            >
                              @{ip.assigned_tenant_username}
                            </span>
                            {ip.assigned_domain && (
                              <span className="text-[11px] text-muted-foreground font-mono">
                                {ip.assigned_domain}
                              </span>
                            )}
                          </div>
                        ) : (
                          <span
                            className="text-xs text-muted-foreground italic"
                            data-testid={`unassigned-label-${ip.id}`}
                          >
                            {t("nodes.ipPools.assignment.unassigned")}
                          </span>
                        )}
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          {/* Allocate dedicated IP (DoD: IP বরাদ্দ করা) */}
                          {ip.type === "dedicated" && !ip.assigned_tenant_id && (
                            <Button
                              variant="outline"
                              size="sm"
                              data-testid={`btn-allocate-ip-${ip.id}`}
                              onClick={() => setAllocatingIp(ip)}
                              className="h-8 px-2 text-xs gap-1 border-primary/30 text-primary hover:bg-primary/10"
                            >
                              <UserCheck className="h-3.5 w-3.5" />
                              <span>{t("nodes.ipPools.actions.allocate")}</span>
                            </Button>
                          )}

                          {/* Release dedicated IP (DoD: IP মুক্ত করা) */}
                          {ip.type === "dedicated" && ip.assigned_tenant_id && (
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={actionIpId === ip.id}
                              data-testid={`btn-release-ip-${ip.id}`}
                              onClick={() => handleReleaseIp(ip)}
                              className="h-8 px-2 text-xs gap-1 text-amber-600 hover:bg-amber-500/10 border-amber-500/30"
                            >
                              <UserMinus className="h-3.5 w-3.5" />
                              <span>{t("nodes.ipPools.actions.release")}</span>
                            </Button>
                          )}

                          {/* Delete IP */}
                          {!ip.is_primary && !ip.assigned_tenant_id && (
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={actionIpId === ip.id}
                              data-testid={`btn-delete-ip-${ip.id}`}
                              onClick={() => handleDeleteIp(ip)}
                              className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </Button>
                          )}
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Add IP Dialog */}
      <AddIpDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        nodes={nodes}
        defaultNodeId={selectedNodeId !== "ALL" ? selectedNodeId : undefined}
        onSave={handleAddIp}
        isSaving={isSaving}
      />

      {/* Allocate IP Dialog (DoD T16.05) */}
      <AllocateIpDialog
        open={Boolean(allocatingIp)}
        onOpenChange={(open) => !open && setAllocatingIp(null)}
        ip={allocatingIp}
        onAllocate={handleAllocateIp}
        isAllocating={isSaving}
      />
    </div>
  );
}
