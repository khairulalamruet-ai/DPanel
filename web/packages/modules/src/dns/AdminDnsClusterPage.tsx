import React, { useState, useEffect } from "react";
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
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Network,
  Plus,
  RefreshCw,
  Server,
  Zap,
  Globe,
  Trash2,
  Power,
  RotateCw,
  CheckCircle2,
  AlertCircle,
  ShieldCheck,
} from "lucide-react";
import {
  DnsClusterServer,
  DnsClusterOverview,
  CreateDnsServerPayload,
} from "./types";
import { AddDnsServerDialog } from "./AddDnsServerDialog";
import { DnsResolutionTestDialog } from "./DnsResolutionTestDialog";

export function AdminDnsClusterPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [servers, setServers] = useState<DnsClusterServer[]>([]);
  const [overview, setOverview] = useState<DnsClusterOverview | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [isAddOpen, setIsAddOpen] = useState(false);
  const [isTestOpen, setIsTestOpen] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [actionServerId, setActionServerId] = useState<string | null>(null);

  const fetchClusterData = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<{
        items: DnsClusterServer[];
        overview: DnsClusterOverview;
      }>("/api/v1/admin/cluster/dns/servers");
      setServers(res.items || []);
      setOverview(res.overview || null);
    } catch (err: any) {
      setError(err?.message || "Failed to load DNS cluster servers");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchClusterData();
  }, []);

  const handleAddServer = async (payload: CreateDnsServerPayload) => {
    setIsSaving(true);
    try {
      const created = await apiFetch<DnsClusterServer>(
        "/api/v1/admin/cluster/dns/servers",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }
      );
      toast({
        title: t("common.appName"),
        description: t("dns.cluster.toast.createSuccess", { name: created.name }),
      });
      setServers((prev) => [...prev, created]);
      if (overview) {
        setOverview({
          ...overview,
          total_servers: overview.total_servers + 1,
          online_servers: overview.online_servers + 1,
        });
      }
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to create DNS server",
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  const handleToggleStatus = async (server: DnsClusterServer) => {
    const nextStatus = server.status === "online" ? "offline" : "online";
    try {
      setActionServerId(server.id);
      const updated = await apiFetch<DnsClusterServer>(
        `/api/v1/admin/cluster/dns/servers/${server.id}`,
        {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ status: nextStatus }),
        }
      );
      setServers((prev) => prev.map((s) => (s.id === server.id ? updated : s)));
      toast({
        title: t("common.appName"),
        description: t("dns.cluster.toast.statusUpdated", {
          name: server.name,
          status: nextStatus,
        }),
      });
      // Refresh overview
      fetchClusterData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to update server status",
        variant: "destructive",
      });
    } finally {
      setActionServerId(null);
    }
  };

  const handleManualSync = async (server: DnsClusterServer) => {
    try {
      setActionServerId(server.id);
      await apiFetch(`/api/v1/admin/cluster/dns/servers/${server.id}/sync`, {
        method: "POST",
      });
      toast({
        title: t("common.appName"),
        description: t("dns.cluster.toast.syncSuccess", { name: server.name }),
      });
      fetchClusterData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Sync failed",
        variant: "destructive",
      });
    } finally {
      setActionServerId(null);
    }
  };

  const handleDeleteServer = async (server: DnsClusterServer) => {
    if (server.role === "primary" && servers.filter((s) => s.role === "primary").length <= 1) {
      toast({
        title: t("common.error"),
        description: t("dns.cluster.toast.cannotDeleteLastPrimary"),
        variant: "destructive",
      });
      return;
    }

    if (!window.confirm(t("dns.cluster.dialog.deleteConfirm", { name: server.name }))) {
      return;
    }

    try {
      setActionServerId(server.id);
      await apiFetch(`/api/v1/admin/cluster/dns/servers/${server.id}`, {
        method: "DELETE",
      });
      toast({
        title: t("common.appName"),
        description: t("dns.cluster.toast.deleteSuccess", { name: server.name }),
      });
      setServers((prev) => prev.filter((s) => s.id !== server.id));
      fetchClusterData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to delete DNS server",
        variant: "destructive",
      });
    } finally {
      setActionServerId(null);
    }
  };

  const renderStatusBadge = (status: DnsClusterServer["status"]) => {
    switch (status) {
      case "online":
        return (
          <Badge
            variant="secondary"
            className="text-[10px] bg-emerald-500/10 text-emerald-600 border border-emerald-500/20 font-medium flex items-center gap-1.5"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
            {t("dns.cluster.status.online")}
          </Badge>
        );
      case "offline":
        return (
          <Badge
            variant="outline"
            className="text-[10px] bg-destructive/10 text-destructive border-destructive/20 font-medium flex items-center gap-1.5"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-destructive" />
            {t("dns.cluster.status.offline")}
          </Badge>
        );
      case "syncing":
        return (
          <Badge
            variant="secondary"
            className="text-[10px] bg-blue-500/10 text-blue-600 border border-blue-500/20 font-medium flex items-center gap-1.5"
          >
            <RefreshCw className="h-2.5 w-2.5 animate-spin" />
            {t("dns.cluster.status.syncing")}
          </Badge>
        );
      case "error":
      default:
        return (
          <Badge
            variant="destructive"
            className="text-[10px] font-medium flex items-center gap-1.5"
          >
            <AlertCircle className="h-2.5 w-2.5" />
            {t("dns.cluster.status.error")}
          </Badge>
        );
    }
  };

  const renderRoleBadge = (role: DnsClusterServer["role"]) => {
    switch (role) {
      case "primary":
        return (
          <Badge variant="default" className="text-[10px] font-medium uppercase tracking-wider">
            {t("dns.cluster.role.primary")}
          </Badge>
        );
      case "secondary":
        return (
          <Badge variant="secondary" className="text-[10px] font-medium uppercase tracking-wider">
            {t("dns.cluster.role.secondary")}
          </Badge>
        );
      case "replica":
      default:
        return (
          <Badge variant="outline" className="text-[10px] font-medium uppercase tracking-wider">
            {t("dns.cluster.role.replica")}
          </Badge>
        );
    }
  };

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Network className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">
              {t("dns.cluster.title")}
            </h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">
            {t("dns.cluster.desc")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchClusterData}
            disabled={isLoading}
            className="gap-2"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.reload")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setIsTestOpen(true)}
            data-testid="btn-open-dns-failover-test"
            className="gap-2 border"
          >
            <Zap className="h-4 w-4 text-amber-500" />
            {t("dns.cluster.testResolutionBtn")}
          </Button>
          <Button
            size="sm"
            onClick={() => setIsAddOpen(true)}
            data-testid="btn-add-dns-server"
            className="gap-2"
          >
            <Plus className="h-4 w-4" />
            {t("dns.cluster.addServerBtn")}
          </Button>
        </div>
      </div>

      {/* Overview Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("dns.cluster.overview.totalServers")}
            </span>
            <Server className="h-4 w-4 text-primary" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold" data-testid="metric-total-dns-servers">
              {overview ? overview.total_servers : servers.length}
            </span>
            <span className="text-xs text-muted-foreground">
              ({overview ? overview.online_servers : servers.filter((s) => s.status === "online").length}{" "}
              {t("dns.cluster.overview.online")})
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("dns.cluster.overview.roles")}
            </span>
            <Network className="h-4 w-4 text-blue-500" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold">
              {overview ? overview.primary_count : servers.filter((s) => s.role === "primary").length}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("dns.cluster.role.primary")} /{" "}
              {overview ? overview.secondary_count : servers.filter((s) => s.role !== "primary").length}{" "}
              {t("dns.cluster.role.secondary")}
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("dns.cluster.overview.syncedZones")}
            </span>
            <Globe className="h-4 w-4 text-emerald-500" />
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-2xl font-bold">
              {overview ? overview.total_zones : 14}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("dns.cluster.overview.zonesCount")}
            </span>
          </div>
        </Card>

        <Card className="p-4 flex flex-col justify-between">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium uppercase tracking-wider">
              {t("dns.cluster.overview.haHealth")}
            </span>
            <ShieldCheck className="h-4 w-4 text-emerald-500" />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <Badge
              variant="secondary"
              className="text-xs font-semibold px-2 py-0.5 bg-emerald-500/10 text-emerald-600 border border-emerald-500/20"
            >
              <CheckCircle2 className="h-3 w-3 mr-1" />
              {t("dns.cluster.overview.haActive")}
            </Badge>
          </div>
        </Card>
      </div>

      {/* Cluster Nameservers Table */}
      <Card>
        <CardHeader className="pb-3 border-b">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base font-semibold">
                {t("dns.cluster.serversTableTitle")}
              </CardTitle>
              <CardDescription className="text-xs mt-0.5">
                {t("dns.cluster.serversTableDesc")}
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
              <Button variant="outline" size="sm" onClick={fetchClusterData}>
                {t("common.retry")}
              </Button>
            </div>
          ) : servers.length === 0 ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              {t("dns.cluster.emptyServers")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[24%]">{t("dns.cluster.table.server")}</TableHead>
                    <TableHead className="w-[12%]">{t("dns.cluster.table.role")}</TableHead>
                    <TableHead className="w-[18%]">{t("dns.cluster.table.network")}</TableHead>
                    <TableHead className="w-[14%]">{t("dns.cluster.table.strategy")}</TableHead>
                    <TableHead className="w-[12%]">{t("dns.cluster.table.status")}</TableHead>
                    <TableHead className="w-[10%]">{t("dns.cluster.table.zones")}</TableHead>
                    <TableHead className="w-[10%] text-right">
                      {t("dns.cluster.table.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {servers.map((server) => (
                    <TableRow
                      key={server.id}
                      data-testid={`dns-server-row-${server.id}`}
                      className="hover:bg-muted/50 transition-colors"
                    >
                      {/* Name & Hostname */}
                      <TableCell>
                        <div className="flex flex-col">
                          <span className="font-semibold text-foreground text-sm">
                            {server.name}
                          </span>
                          <span className="font-mono text-xs text-muted-foreground mt-0.5">
                            {server.hostname}
                          </span>
                          {server.version && (
                            <span className="text-[10px] text-muted-foreground font-mono mt-0.5">
                              {server.version}
                            </span>
                          )}
                        </div>
                      </TableCell>

                      {/* Role */}
                      <TableCell>{renderRoleBadge(server.role)}</TableCell>

                      {/* IP & API */}
                      <TableCell>
                        <div className="flex flex-col text-xs font-mono">
                          <span className="text-foreground">
                            {server.ip_address}:{server.port}
                          </span>
                          <span className="text-[11px] text-muted-foreground truncate max-w-[180px]">
                            {server.api_url}
                          </span>
                        </div>
                      </TableCell>

                      {/* Strategy */}
                      <TableCell>
                        <span className="text-xs font-medium capitalize">
                          {server.sync_strategy.replace("_", " ")}
                        </span>
                        <div className="text-[11px] text-muted-foreground">
                          {server.latency_ms} ms
                        </div>
                      </TableCell>

                      {/* Status */}
                      <TableCell>
                        <div className="flex flex-col gap-1 items-start">
                          {renderStatusBadge(server.status)}
                        </div>
                      </TableCell>

                      {/* Zones count */}
                      <TableCell>
                        <span className="font-mono text-xs font-medium">
                          {server.zones_count}
                        </span>
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          {/* Sync Now */}
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={actionServerId === server.id || server.status === "offline"}
                            onClick={() => handleManualSync(server)}
                            data-testid={`sync-dns-server-${server.id}`}
                            title={t("dns.cluster.actions.syncNow")}
                            className="h-8 w-8 p-0"
                          >
                            <RotateCw
                              className={`h-3.5 w-3.5 ${
                                actionServerId === server.id ? "animate-spin" : ""
                              }`}
                            />
                          </Button>

                          {/* Toggle Offline / Online */}
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={actionServerId === server.id}
                            onClick={() => handleToggleStatus(server)}
                            data-testid={`toggle-dns-status-${server.id}`}
                            title={
                              server.status === "online"
                                ? t("dns.cluster.actions.takeOffline")
                                : t("dns.cluster.actions.bringOnline")
                            }
                            className={`h-8 w-8 p-0 ${
                              server.status === "online"
                                ? "text-amber-600 hover:bg-amber-500/10"
                                : "text-emerald-600 hover:bg-emerald-500/10"
                            }`}
                          >
                            <Power className="h-3.5 w-3.5" />
                          </Button>

                          {/* Delete */}
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={actionServerId === server.id || server.role === "primary"}
                            onClick={() => handleDeleteServer(server)}
                            data-testid={`delete-dns-server-${server.id}`}
                            title={t("common.delete")}
                            className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10 disabled:opacity-40"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
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

      {/* Add DNS Server Dialog */}
      <AddDnsServerDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        onSave={handleAddServer}
        isSaving={isSaving}
      />

      {/* Resolution & Failover Test Dialog */}
      <DnsResolutionTestDialog
        open={isTestOpen}
        onOpenChange={setIsTestOpen}
        servers={servers}
      />
    </div>
  );
}
