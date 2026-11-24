import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Input,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import {
  Network,
  Plus,
  Key,
  Copy,
  Check,
  Server,
  Terminal,
  Activity,
  ShieldCheck,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ClusterNodeSummary, JoinCodeResult } from "../types";
import { joinServer, generateJoinCode, listClusterNodes, pingNode } from "../api";
import { SudoConfirmModal } from "../SudoConfirmModal";

interface MultiServerTabProps {
  canManage: boolean;
  onAuditActionTriggered?: () => void;
}

export function MultiServerTab({ canManage, onAuditActionTriggered }: MultiServerTabProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [nodes, setNodes] = useState<ClusterNodeSummary[]>([]);
  const [isLoadingNodes, setIsLoadingNodes] = useState(false);

  // Join form state
  const [remoteAddress, setRemoteAddress] = useState("");
  const [remoteName, setRemoteName] = useState("");
  const [joinCodeInput, setJoinCodeInput] = useState("");
  const [isJoinSubmitting, setIsJoinSubmitting] = useState(false);

  // Generated join code & install command state
  const [generatedJoin, setGeneratedJoin] = useState<JoinCodeResult | null>(null);
  const [isGenerating, setIsGenerating] = useState(false);
  const [copiedCode, setCopiedCode] = useState(false);
  const [copiedCmd, setCopiedCmd] = useState(false);

  // Sudo confirmation modal
  const [isSudoModalOpen, setIsSudoModalOpen] = useState(false);

  const fetchNodes = async () => {
    try {
      setIsLoadingNodes(true);
      const list = await listClusterNodes();
      setNodes(list);
    } catch {
      // ignore
    } finally {
      setIsLoadingNodes(false);
    }
  };

  useEffect(() => {
    void fetchNodes();
  }, []);

  const handleGenerateJoinCode = async () => {
    try {
      setIsGenerating(true);
      const res = await generateJoinCode();
      setGeneratedJoin(res);
      toast({
        title: t("common.success"),
        description: t("system.multi.codeGeneratedSuccess", "New cluster join code generated (valid for 15 minutes)."),
      });
      onAuditActionTriggered?.();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsGenerating(false);
    }
  };

  const handleCopy = (text: string, type: "code" | "cmd") => {
    navigator.clipboard.writeText(text);
    if (type === "code") {
      setCopiedCode(true);
      setTimeout(() => setCopiedCode(false), 2000);
    } else {
      setCopiedCmd(true);
      setTimeout(() => setCopiedCmd(false), 2000);
    }
    toast({
      title: t("common.copied", "Copied to clipboard"),
    });
  };

  const handlePingNode = async (nodeId: string) => {
    try {
      const res = await pingNode(nodeId);
      toast({
        title: res.ok ? t("common.success") : t("common.error"),
        description: res.ok
          ? t("system.multi.pingSuccess", "Node heartbeat responded successfully.")
          : t("system.multi.pingFailure", "Node did not respond to ping."),
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleJoinServer = async (sudoPassword: string) => {
    try {
      setIsJoinSubmitting(true);
      await joinServer(
        {
          address: remoteAddress.trim(),
          name: remoteName.trim(),
          join_code: joinCodeInput.trim(),
        },
        sudoPassword,
      );
      toast({
        title: t("common.success"),
        description: t("system.multi.joinSuccess", "Successfully connected and joined new server node."),
      });
      setRemoteAddress("");
      setRemoteName("");
      setJoinCodeInput("");
      await fetchNodes();
      onAuditActionTriggered?.();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsJoinSubmitting(false);
    }
  };

  const canJoin =
    remoteAddress.trim().length > 0 &&
    remoteName.trim().length > 0 &&
    joinCodeInput.trim().length > 0 &&
    canManage;

  return (
    <div className="space-y-6">
      {/* 1. Generate Join Code & Install Command */}
      <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
        <Card data-testid="card-generate-join-code">
          <CardHeader>
            <CardTitle className="text-base font-semibold flex items-center gap-2">
              <Key className="h-4 w-4 text-primary" />
              <span>{t("system.multi.generateCodeTitle", "Generate Join Code for this Server")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground">
              {t(
                "system.multi.generateCodeDesc",
                "Create a secure, time-limited enrollment token so another node agent can join this control plane",
              )}
            </CardDescription>
          </CardHeader>

          <CardContent className="space-y-4">
            {generatedJoin ? (
              <div className="space-y-3 rounded-lg border bg-muted/30 p-4">
                <div className="space-y-1">
                  <span className="text-[11px] font-medium uppercase text-muted-foreground">
                    {t("system.multi.joinCodeLabel", "Active Join Token (Expires in 15 min)")}
                  </span>
                  <div className="flex items-center gap-2">
                    <Input
                      readOnly
                      data-testid="input-generated-code"
                      value={generatedJoin.code}
                      className="font-mono text-xs bg-background"
                    />
                    <Button
                      size="sm"
                      variant="outline"
                      data-testid="btn-copy-code"
                      onClick={() => handleCopy(generatedJoin.code, "code")}
                    >
                      {copiedCode ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                    </Button>
                  </div>
                </div>

                <div className="space-y-1">
                  <span className="text-[11px] font-medium uppercase text-muted-foreground">
                    {t("system.multi.installCmdLabel", "One-Line Node Install Command")}
                  </span>
                  <div className="flex items-center gap-2">
                    <Input
                      readOnly
                      data-testid="input-generated-cmd"
                      value={generatedJoin.install_command}
                      className="font-mono text-xs bg-background"
                    />
                    <Button
                      size="sm"
                      variant="outline"
                      data-testid="btn-copy-cmd"
                      onClick={() => handleCopy(generatedJoin.install_command, "cmd")}
                    >
                      {copiedCmd ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                    </Button>
                  </div>
                </div>
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">
                {t(
                  "system.multi.noCodeYet",
                  "Click the button below to generate a new cryptographic join token and ready-to-run installation shell command.",
                )}
              </p>
            )}
          </CardContent>

          <CardFooter className="border-t px-6 py-3 flex justify-end">
            <Button
              size="sm"
              disabled={!canManage || isGenerating}
              data-testid="btn-generate-join-code"
              onClick={handleGenerateJoinCode}
              className="gap-1.5 text-xs"
            >
              {isGenerating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Key className="h-3.5 w-3.5" />}
              <span>{t("system.multi.generateBtn", "Generate Join Code")}</span>
            </Button>
          </CardFooter>
        </Card>

        {/* 2. Add / Join an External Server */}
        <Card data-testid="card-join-server">
          <CardHeader>
            <CardTitle className="text-base font-semibold flex items-center gap-2">
              <Network className="h-4 w-4 text-primary" />
              <span>{t("system.multi.joinServerTitle", "Add a Server with Join Code")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground">
              {t(
                "system.multi.joinServerDesc",
                "Enroll an external server into this cluster using its public host address and invitation code",
              )}
            </CardDescription>
          </CardHeader>

          <CardContent className="space-y-3">
            <div className="space-y-1">
              <label className="text-xs font-medium text-foreground">
                {t("system.multi.nodeAddress", "Server IP / Address")}
              </label>
              <Input
                data-testid="input-join-address"
                placeholder="198.51.100.40 or node2.example.com"
                value={remoteAddress}
                onChange={(e) => setRemoteAddress(e.target.value)}
                className="font-mono text-xs"
              />
            </div>

            <div className="space-y-1">
              <label className="text-xs font-medium text-foreground">
                {t("system.multi.nodeName", "Server Name / Identifier")}
              </label>
              <Input
                data-testid="input-join-name"
                placeholder="node-frankfurt-02"
                value={remoteName}
                onChange={(e) => setRemoteName(e.target.value)}
                className="text-xs"
              />
            </div>

            <div className="space-y-1">
              <label className="text-xs font-medium text-foreground">
                {t("system.multi.nodeJoinCode", "Join Code / Enrollment Token")}
              </label>
              <Input
                data-testid="input-join-code"
                placeholder="dhn_join_..."
                value={joinCodeInput}
                onChange={(e) => setJoinCodeInput(e.target.value)}
                className="font-mono text-xs"
              />
            </div>
          </CardContent>

          <CardFooter className="border-t px-6 py-3 flex justify-end">
            <Button
              size="sm"
              disabled={!canJoin || isJoinSubmitting}
              data-testid="btn-submit-join"
              onClick={() => setIsSudoModalOpen(true)}
              className="gap-1.5 text-xs"
            >
              {isJoinSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Plus className="h-3.5 w-3.5" />}
              <span>{t("system.multi.connectServerBtn", "Add & Connect Server")}</span>
            </Button>
          </CardFooter>
        </Card>
      </div>

      {/* 3. Cluster Nodes Table */}
      <Card data-testid="card-cluster-nodes">
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="text-base font-semibold flex items-center gap-2">
                <Server className="h-4 w-4 text-primary" />
                <span>{t("system.multi.clusterNodesTitle", "Enrolled Cluster Nodes")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground">
                {t("system.multi.clusterNodesDesc", "Overview of active servers connected to this BuiltPanel control plane")}
              </CardDescription>
            </div>
            <Button
              size="sm"
              variant="outline"
              onClick={fetchNodes}
              disabled={isLoadingNodes}
              className="gap-1.5 text-xs"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isLoadingNodes ? "animate-spin" : ""}`} />
              <span>{t("common.refresh", "Refresh")}</span>
            </Button>
          </div>
        </CardHeader>

        <CardContent>
          {nodes.length === 0 ? (
            <p className="py-6 text-center text-xs text-muted-foreground">
              {t("system.multi.noNodesFound", "No cluster nodes enrolled.")}
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="text-xs">{t("common.name", "Node Name")}</TableHead>
                    <TableHead className="text-xs">{t("system.multi.role", "Role")}</TableHead>
                    <TableHead className="text-xs">{t("system.multi.ip", "Public IP / Host")}</TableHead>
                    <TableHead className="text-xs">{t("system.multi.version", "Agent Version")}</TableHead>
                    <TableHead className="text-xs">{t("common.status", "Status")}</TableHead>
                    <TableHead className="text-xs text-right">{t("common.actions", "Actions")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {nodes.map((n) => (
                    <TableRow key={n.id} data-testid={`node-row-${n.id}`}>
                      <TableCell className="font-semibold text-xs text-foreground">
                        {n.name}
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline" className="text-[10px] uppercase font-mono">
                          {n.role}
                        </Badge>
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {n.ip_address || n.hostname}
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        v{n.agent_version}
                      </TableCell>
                      <TableCell>
                        <Badge
                          variant={n.status === "online" ? "default" : "destructive"}
                          className="text-[10px] capitalize"
                        >
                          {n.status}
                        </Badge>
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid={`btn-ping-node-${n.id}`}
                          onClick={() => handlePingNode(n.id)}
                          className="h-7 px-2 text-xs gap-1"
                        >
                          <Activity className="h-3.5 w-3.5 text-primary" />
                          <span>{t("system.multi.ping", "Ping")}</span>
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Sudo Modal for Joining a Server */}
      <SudoConfirmModal
        open={isSudoModalOpen}
        onOpenChange={setIsSudoModalOpen}
        title={t("system.multi.sudoConfirmTitle", "Confirm Server Enrollment")}
        description={t(
          "system.multi.sudoConfirmDesc",
          "Connecting an external node establishes mutual TLS trust and node agent registration. Enter administrator password to proceed.",
        )}
        confirmLabel={t("system.multi.connectServerBtn", "Add & Connect Server")}
        onConfirm={handleJoinServer}
      />
    </div>
  );
}
