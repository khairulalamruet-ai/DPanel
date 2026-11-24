import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import {
  Server,
  RefreshCw,
  ShieldAlert,
  Clock,
  Cpu,
  Globe,
  Terminal,
  Power,
  HardDrive,
  ArrowRightLeft,
  Network,
  Sliders,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { useSession } from "@builtpanel/auth";
import {
  listNodeChoices,
  loadServer,
  loadAudit,
  setTimezone,
  setSwap,
  dropCaches,
  type NodeChoice,
} from "./api";
import { ServerOverview, ServerAuditEvent } from "./types";
import { ServerTimeTab } from "./tabs/ServerTimeTab";
import { SshAccessTab } from "./tabs/SshAccessTab";
import { ServerRebootTab } from "./tabs/ServerRebootTab";
import { SwapTab } from "./tabs/SwapTab";
import { ServerMigrateTab } from "./tabs/ServerMigrateTab";
import { MultiServerTab } from "./tabs/MultiServerTab";
import { ServerAuditList } from "./ServerAuditList";

export type SystemTabKey = "time" | "ssh" | "reboot" | "swap" | "migrate" | "multi-server";

export function AdminSystemPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const { hasPermission } = useSession();

  const canRead = hasPermission("admin.server.read") || hasPermission("admin.server.manage");
  const canManage = hasPermission("admin.server.manage");

  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const [selectedNode, setSelectedNode] = useState("");
  const [activeTab, setActiveTab] = useState<SystemTabKey>("time");
  const [overview, setOverview] = useState<ServerOverview | null>(null);
  const [auditEvents, setAuditEvents] = useState<ServerAuditEvent[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  // Initialize nodes list
  useEffect(() => {
    listNodeChoices()
      .then((list) => {
        setNodes(list);
        setSelectedNode((current) => current || list[0]?.id || "");
      })
      .catch((err: any) =>
        toast({
          title: t("common.error"),
          description: err?.message || t("common.unknownError"),
          variant: "destructive",
        }),
      );
  }, []);

  const fetchData = useCallback(async () => {
    if (!selectedNode) return;
    try {
      setIsLoading(true);
      const [server, auditData] = await Promise.all([
        loadServer(selectedNode),
        loadAudit(selectedNode).catch(() => []),
      ]);
      setOverview(server.overview);
      setAuditEvents(auditData);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [selectedNode]);

  useEffect(() => {
    void fetchData();
  }, [fetchData]);

  // Handlers for Server Time Tab
  const handleUpdateTimezone = async (timezone: string, sudoPassword: string) => {
    try {
      await setTimezone(selectedNode, timezone, sudoPassword);
      toast({
        title: t("common.success"),
        description: t("serverTools.timezoneUpdatedSuccess", { timezone }),
      });
      await fetchData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    }
  };

  // Handlers for Swap Tab
  const handleUpdateSwap = async (totalMb: number, _swappiness: number, sudoPassword: string) => {
    try {
      await setSwap(selectedNode, totalMb, sudoPassword);
      toast({
        title: t("common.success"),
        description: t("serverTools.swapUpdatedSuccess", { totalMb, swappiness: 0 }),
      });
      await fetchData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    }
  };

  const handleDropCaches = async (sudoPassword: string) => {
    try {
      await dropCaches(selectedNode, sudoPassword);
      toast({
        title: t("common.success"),
        description: t("serverTools.dropCachesDone", "The page cache was dropped"),
      });
      await fetchData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    }
  };

  const currentNodeName = nodes.find((n) => n.id === selectedNode)?.name || selectedNode;

  if (!canRead) {
    return (
      <div className="py-12 text-center" data-testid="permission-denied-server">
        <h2 className="text-lg font-semibold text-foreground">
          {t("common.permissionDenied", "Permission Denied")}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("system.permissionRequired", "You require 'admin.server.read' privileges to view server tools.")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Server className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight" data-testid="system-page-title">
              {t("system.pageTitle", "System Administration")}
            </h1>
            <Badge variant="outline" className="gap-1 border-amber-500/30 text-amber-600 dark:text-amber-400">
              <ShieldAlert className="h-3 w-3" />
              <span>{t("serverTools.sudoModeRequired", "Sudo Mode Required")}</span>
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("system.pageSubtitle", "Configure timezone, manage SSH security, reboot nodes, adjust swap, execute migrations, and manage cluster servers")}
          </p>
        </div>

        {/* Node selector & Refresh */}
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5 rounded-md border bg-background px-3 py-1.5 text-xs shadow-sm">
            <Server className="h-3.5 w-3.5 text-muted-foreground" />
            <select
              data-testid="select-node"
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
            onClick={fetchData}
            disabled={isLoading}
            data-testid="btn-refresh-system"
            className="gap-1 text-xs"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh", "Refresh")}</span>
          </Button>
        </div>
      </div>

      {/* Node Overview Banner */}
      {overview && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4" data-testid="card-server-overview">
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.hostname", "Hostname")}</span>
            <p className="font-mono text-sm font-semibold truncate" data-testid="text-node-hostname">{overview.hostname}</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.kernelOs", "OS / Kernel")}</span>
            <p className="font-mono text-sm font-semibold truncate">{overview.os} ({overview.kernel})</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.uptime", "System Uptime")}</span>
            <p className="flex items-center gap-1 text-sm font-semibold text-emerald-600 dark:text-emerald-400">
              <Clock className="h-3.5 w-3.5" />
              <span>{overview.uptime_human}</span>
            </p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.memoryOverview", "Available Memory")}</span>
            <p className="flex items-center gap-1 font-mono text-sm font-semibold">
              <Cpu className="h-3.5 w-3.5 text-primary" />
              <span>{overview.memory.available_mb} / {overview.memory.total_mb} MB</span>
            </p>
          </div>
        </div>
      )}

      {/* Six Primary Tabs */}
      <div className="border-b flex flex-wrap gap-2 sm:gap-4" role="tablist">
        {/* Tab 1: Server time */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "time"}
          data-testid="tab-system-time"
          onClick={() => setActiveTab("time")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "time"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Globe className="h-4 w-4" />
          <span>{t("system.tabs.time", "Server time")}</span>
        </button>

        {/* Tab 2: SSH access */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "ssh"}
          data-testid="tab-system-ssh"
          onClick={() => setActiveTab("ssh")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "ssh"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Terminal className="h-4 w-4" />
          <span>{t("system.tabs.ssh", "SSH access")}</span>
        </button>

        {/* Tab 3: Server reboot */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "reboot"}
          data-testid="tab-system-reboot"
          onClick={() => setActiveTab("reboot")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "reboot"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Power className="h-4 w-4" />
          <span>{t("system.tabs.reboot", "Server reboot")}</span>
        </button>

        {/* Tab 4: Swap */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "swap"}
          data-testid="tab-system-swap"
          onClick={() => setActiveTab("swap")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "swap"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <HardDrive className="h-4 w-4" />
          <span>{t("system.tabs.swap", "Swap")}</span>
        </button>

        {/* Tab 5: Migrate */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "migrate"}
          data-testid="tab-system-migrate"
          onClick={() => setActiveTab("migrate")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "migrate"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <ArrowRightLeft className="h-4 w-4" />
          <span>{t("system.tabs.migrate", "Migrate")}</span>
        </button>

        {/* Tab 6: Multi-server */}
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "multi-server"}
          data-testid="tab-system-multi-server"
          onClick={() => setActiveTab("multi-server")}
          className={`pb-2.5 px-2 text-xs sm:text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "multi-server"
              ? "border-primary text-primary font-semibold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Network className="h-4 w-4" />
          <span>{t("system.tabs.multiServer", "Multi-server")}</span>
        </button>
      </div>

      {/* Tab Panels */}
      <div>
        {activeTab === "time" && (
          <ServerTimeTab
            currentTimezone={overview?.timezone || "UTC"}
            currentTimeStr={overview?.current_time || new Date().toISOString()}
            canManage={canManage}
            onUpdateTimezone={handleUpdateTimezone}
          />
        )}

        {activeTab === "ssh" && (
          <SshAccessTab
            nodeId={selectedNode}
            canManage={canManage}
            onAuditActionTriggered={fetchData}
          />
        )}

        {activeTab === "reboot" && (
          <ServerRebootTab
            nodeId={selectedNode}
            nodeName={currentNodeName}
            nodeHostname={overview?.hostname || currentNodeName}
            canManage={canManage}
            onAuditActionTriggered={fetchData}
          />
        )}

        {activeTab === "swap" && overview && (
          <SwapTab
            swap={overview.swap}
            memory={overview.memory}
            canManage={canManage}
            onUpdateSwap={handleUpdateSwap}
            onDropCaches={handleDropCaches}
          />
        )}

        {activeTab === "migrate" && (
          <ServerMigrateTab
            nodeId={selectedNode}
            canManage={canManage}
            onAuditActionTriggered={fetchData}
          />
        )}

        {activeTab === "multi-server" && (
          <MultiServerTab
            canManage={canManage}
            onAuditActionTriggered={fetchData}
          />
        )}
      </div>

      {/* Audit Log Table (DoD: Every action recorded in audit log) */}
      <ServerAuditList events={auditEvents} isLoading={isLoading} />
    </div>
  );
}

// Keep AdminServerToolsPage as an alias for backwards compatibility
export const AdminServerToolsPage = AdminSystemPage;
