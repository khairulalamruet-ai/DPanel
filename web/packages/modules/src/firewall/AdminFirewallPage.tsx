import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  Input,
  useToast,
} from "@builtpanel/ui";
import {
  ShieldAlert,
  ShieldCheck,
  Network,
  Ban,
  Server,
  RefreshCw,
  Lock,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { listNodeChoices, type NodeChoice } from "../nodes/api";
import { loadFirewall, openPort, closePort, blockAddress, unblockAddress } from "./api";
import { AdminFirewallOverview, NodePortRule, GlobalIpBan } from "./types";
import { AdminPortsTable } from "./AdminPortsTable";
import { AdminBansTable } from "./AdminBansTable";

export function AdminFirewallPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const [selectedNode, setSelectedNode] = useState("");
  const [activeTab, setActiveTab] = useState<"ports" | "bans">("ports");
  const [overview, setOverview] = useState<AdminFirewallOverview | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    listNodeChoices()
      .then((list) => {
        setNodes(list);
        setSelectedNode((current) => current || list[0]?.id || "");
      })
      .catch((err: any) =>
        toast({ title: t("common.error"), description: err?.message || t("common.unknownError"), variant: "destructive" }),
      );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const fetchOverview = useCallback(async () => {
    if (!selectedNode) return;
    try {
      setIsLoading(true);
      setOverview((await loadFirewall(selectedNode)).overview);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [selectedNode, toast, t]);

  useEffect(() => {
    fetchOverview();
  }, [fetchOverview]);

  const handleOpenPort = async (port: number, protocol: "tcp" | "udp", _serviceName: string) => {
    try {
      await openPort(selectedNode, port, protocol);
      toast({
        title: t("common.success"),
        description: t("firewall.portOpenedSuccess", { port, protocol }),
      });
      await fetchOverview();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    }
  };

  const handleClosePort = async (portId: string, port: number) => {
    try {
      await closePort(selectedNode, portId);
      toast({
        title: t("common.success"),
        description: t("firewall.portClosedSuccess", { port }),
      });
      await fetchOverview();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleAddBan = async (ip: string, _reason: string, _durationHours: number) => {
    try {
      await blockAddress(selectedNode, ip);
      toast({
        title: t("common.success"),
        description: t("firewall.ipBannedSuccess", { ip }),
      });
      await fetchOverview();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    }
  };

  const handleUnban = async (_banId: string, ip: string) => {
    try {
      await unblockAddress(selectedNode, ip);
      toast({
        title: t("common.success"),
        description: t("firewall.ipUnbannedSuccess", { ip }),
      });
      await fetchOverview();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <ShieldAlert className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-firewall-title">
              {t("firewall.adminPageTitle")}
            </h1>
            <Badge variant="outline" className="border-emerald-500/30 text-emerald-600 dark:text-emerald-400 gap-1">
              <ShieldCheck className="h-3 w-3" />
              <span>nftables (Kernel Layer)</span>
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("firewall.adminPageSubtitle")}
          </p>
        </div>

        {/* Node Selector & Refresh */}
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5 rounded-md border bg-background px-3 py-1.5 text-xs shadow-sm">
            <Server className="h-3.5 w-3.5 text-muted-foreground" />
            <select
              data-testid="select-admin-firewall-node"
              value={selectedNode}
              onChange={(e) => setSelectedNode(e.target.value)}
              className="bg-transparent font-medium focus:outline-none"
            >
              {nodes.map((n) => (
                <option key={n.id} value={n.id}>
                  {n.name}
                </option>
              ))}
            </select>
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={fetchOverview}
            disabled={isLoading}
            data-testid="btn-refresh-admin-firewall"
            className="gap-1 text-xs"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {/* Node Overview Stats Cards */}
      {overview && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4" data-testid="card-admin-firewall-stats">
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.firewallEngine")}</span>
            <p className="font-mono text-sm font-semibold truncate">{overview.nftables_version}</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.openPortsCount")}</span>
            <p className="font-mono text-sm font-semibold text-primary">{overview.active_ports.length} ports open</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.activeBansCount")}</span>
            <p className="font-mono text-sm font-semibold text-destructive">{overview.active_bans.length} IPs banned</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.sshLockoutDefense")}</span>
            <p className="flex items-center gap-1 text-xs font-semibold text-emerald-600 dark:text-emerald-400">
              <Lock className="h-3 w-3" />
              <span>SSH Port 22 Protected</span>
            </p>
          </div>
        </div>
      )}

      {/* Tabs */}
      <div className="flex border-b border-border">
        <button
          type="button"
          onClick={() => setActiveTab("ports")}
          data-testid="tab-firewall-ports"
          className={`flex items-center gap-2 border-b-2 px-4 py-2.5 text-xs font-medium transition-colors ${
            activeTab === "ports"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Network className="h-4 w-4" />
          <span>{t("firewall.inboundPortsTab")}</span>
          <Badge variant="secondary" className="font-mono text-[10px] ml-1">
            {overview?.active_ports.length ?? 0}
          </Badge>
        </button>

        <button
          type="button"
          onClick={() => setActiveTab("bans")}
          data-testid="tab-firewall-bans"
          className={`flex items-center gap-2 border-b-2 px-4 py-2.5 text-xs font-medium transition-colors ${
            activeTab === "bans"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Ban className="h-4 w-4" />
          <span>{t("firewall.ipBansTab")}</span>
          <Badge variant="secondary" className="font-mono text-[10px] ml-1">
            {overview?.active_bans.length ?? 0}
          </Badge>
        </button>
      </div>

      {/* Tab Contents */}
      {activeTab === "ports" && overview && (
        <AdminPortsTable
          ports={overview.active_ports}
          onOpenPort={handleOpenPort}
          onClosePort={handleClosePort}
          isLoading={isLoading}
        />
      )}

      {activeTab === "bans" && overview && (
        <AdminBansTable
          bans={overview.active_bans}
          onAddBan={handleAddBan}
          onUnban={handleUnban}
          isLoading={isLoading}
        />
      )}
    </div>
  );
}
