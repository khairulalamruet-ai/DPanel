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
  Wrench,
  Server,
  RefreshCw,
  Power,
  ShieldAlert,
  Clock,
  Cpu,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { listNodeChoices, loadServer, loadAudit, setTimezone, setSwap, dropCaches, reboot, type NodeChoice } from "./api";
import { ServerOverview, SshConfigInfo, ServerAuditEvent } from "./types";
import { TimezoneCard } from "./TimezoneCard";
import { SwapCard } from "./SwapCard";
import { DropCachesCard } from "./DropCachesCard";
import { SshConfigCard } from "./SshConfigCard";
import { RebootDialog } from "./RebootDialog";
import { ServerAuditList } from "./ServerAuditList";

export function AdminServerToolsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const [selectedNode, setSelectedNode] = useState("");
  const [overview, setOverview] = useState<ServerOverview | null>(null);
  const [sshConfig, setSshConfig] = useState<SshConfigInfo | null>(null);
  const [auditEvents, setAuditEvents] = useState<ServerAuditEvent[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isRebootOpen, setIsRebootOpen] = useState(false);

  useEffect(() => {
    listNodeChoices()
      .then((list) => {
        setNodes(list);
        setSelectedNode((current) => current || list[0]?.id || "");
      })
      .catch((err: any) =>
        toast({ title: t("common.error"), description: err?.message || t("common.unknownError"), variant: "destructive" }),
      );
  }, []);

  const fetchData = useCallback(async () => {
    if (!selectedNode) return;
    try {
      setIsLoading(true);
      const [server, auditData] = await Promise.all([loadServer(selectedNode), loadAudit(selectedNode).catch(() => [])]);
      setOverview(server.overview);
      setSshConfig(server.ssh);
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
  }, [selectedNode, toast, t]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

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

  const handleReboot = async (sudoPassword: string) => {
    try {
      await reboot(selectedNode, nodes.find((n) => n.id === selectedNode)?.name ?? "", sudoPassword);
      toast({
        title: t("serverTools.rebootInitiatedTitle"),
        description: t("serverTools.rebootInitiatedDescription"),
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

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Wrench className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight" data-testid="server-tools-title">
              {t("serverTools.pageTitle")}
            </h1>
            <Badge variant="outline" className="gap-1 border-amber-500/30 text-amber-600 dark:text-amber-400">
              <ShieldAlert className="h-3 w-3" />
              <span>{t("serverTools.sudoModeRequired")}</span>
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("serverTools.pageSubtitle")}
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
            data-testid="btn-refresh-server-tools"
            className="gap-1 text-xs"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {/* Node Overview Banner */}
      {overview && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4" data-testid="card-server-overview">
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.hostname")}</span>
            <p className="font-mono text-sm font-semibold truncate" data-testid="text-node-hostname">{overview.hostname}</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.kernelOs")}</span>
            <p className="font-mono text-sm font-semibold truncate">{overview.os} ({overview.kernel})</p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.uptime")}</span>
            <p className="flex items-center gap-1 text-sm font-semibold text-emerald-600 dark:text-emerald-400">
              <Clock className="h-3.5 w-3.5" />
              <span>{overview.uptime_human}</span>
            </p>
          </div>
          <div className="rounded-lg border bg-card p-3 shadow-sm">
            <span className="text-[11px] text-muted-foreground uppercase">{t("serverTools.memoryOverview")}</span>
            <p className="flex items-center gap-1 font-mono text-sm font-semibold">
              <Cpu className="h-3.5 w-3.5 text-primary" />
              <span>{overview.memory.available_mb} / {overview.memory.total_mb} MB</span>
            </p>
          </div>
        </div>
      )}

      {/* Primary Configuration Grid */}
      <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
        {/* 1. Timezone */}
        <TimezoneCard
          currentTimezone={overview?.timezone || "UTC"}
          currentTimeStr={overview?.current_time || new Date().toISOString()}
          onUpdateTimezone={handleUpdateTimezone}
        />

        {/* 2. Swap Management */}
        {overview && (
          <SwapCard
            swap={overview.swap}
            onUpdateSwap={handleUpdateSwap}
          />
        )}

        {/* 3. Drop Caches */}
        {overview && (
          <DropCachesCard
            memory={overview.memory}
            onDropCaches={handleDropCaches}
          />
        )}

        {/* 4. SSH Configuration */}
        <SshConfigCard
          sshConfig={sshConfig}
          isLoading={isLoading}
        />
      </div>

      {/* Danger Zone: Reboot */}
      <Card className="border-destructive/30 bg-destructive/5" data-testid="card-danger-reboot">
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="flex items-center gap-2 text-base text-destructive">
                <Power className="h-4 w-4" />
                <span data-testid="title-reboot-server">{t("serverTools.rebootSectionTitle")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground">
                {t("serverTools.rebootSectionDescription")}
              </CardDescription>
            </div>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setIsRebootOpen(true)}
              data-testid="btn-open-reboot"
              className="gap-1.5 text-xs"
            >
              <Power className="h-3.5 w-3.5" />
              {t("serverTools.rebootButton")}
            </Button>
          </div>
        </CardHeader>
      </Card>

      {/* Audit Log Table (DoD: প্রতিটা কাজ audit-এ যায়) */}
      <ServerAuditList events={auditEvents} isLoading={isLoading} />

      {/* Reboot confirmation modal */}
      <RebootDialog
        open={isRebootOpen}
        onOpenChange={setIsRebootOpen}
        nodeHostname={overview?.hostname || ""}
        confirmWord={nodes.find((n) => n.id === selectedNode)?.name}
        onConfirm={handleReboot}
      />
    </div>
  );
}
