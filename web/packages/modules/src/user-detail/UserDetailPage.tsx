import React, { useState, useEffect } from "react";
import { useParams } from "@tanstack/react-router";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { loadUserDetail } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  User,
  Globe,
  Database,
  FileText,
  Activity,
  Box,
  Cpu,
  Layers,
  KeyRound,
  ShieldAlert,
  ShieldCheck,
  LogIn,
  Trash2,
  RefreshCw,
  ExternalLink,
  ArrowLeft,
  Terminal,
} from "lucide-react";
import { UserDetail, UserDetailTab } from "./types";
import { ResetPasswordDialog } from "./ResetPasswordDialog";

function formatBytes(mb: number): string {
  if (mb === -1) return "∞ Unlimited";
  if (mb < 1024) return `${mb} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

export function UserDetailPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  // Extract user id from router params or default to alpha
  const params = useParams({ strict: false }) as any;
  const userId = params?.id || "usr_0001";

  const [activeTab, setActiveTab] = useState<UserDetailTab>("overview");
  const [user, setUser] = useState<UserDetail | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  // Dialogs
  const [isResetPassOpen, setIsResetPassOpen] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const fetchUserDetail = async () => {
    setIsLoading(true);
    try {
      const data = await loadUserDetail(userId);
      setUser(data);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userDetail.toast.loadError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchUserDetail();
  }, [userId]);

  const handleImpersonate = async () => {
    if (!user) return;
    try {
      const res = await apiFetch<{ token: string; redirect_url: string }>(
        `/api/v1/admin/users/${user.id}/impersonate`,
        { method: "POST" }
      );
      toast({
        title: t("userDetail.toast.impersonateSuccess", { username: user.username }),
        description: t("userDetail.toast.impersonateDesc"),
      });
      if (res.redirect_url) {
        window.open(res.redirect_url, "_blank");
      }
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userDetail.toast.impersonateError"),
        variant: "destructive",
      });
    }
  };

  const handleToggleSuspend = async () => {
    if (!user) return;
    const isSuspending = user.status !== "suspended";
    const endpoint = `/api/v1/admin/users/${user.id}/${isSuspending ? "suspend" : "unsuspend"}`;

    try {
      await apiFetch(endpoint, { method: "POST" });
      toast({
        title: t("common.appName"),
        description: isSuspending
          ? t("userDetail.toast.suspendSuccess", { username: user.username })
          : t("userDetail.toast.unsuspendSuccess", { username: user.username }),
      });
      fetchUserDetail();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userDetail.toast.statusError"),
        variant: "destructive",
      });
    }
  };

  const handleResetPassword = async (newPassword: string) => {
    if (!user) return;
    setIsSubmitting(true);
    try {
      await apiFetch(`/api/v1/admin/users/${user.id}/password`, {
        method: "POST",
        body: JSON.stringify({ password: newPassword }),
      });
      toast({
        title: t("common.appName"),
        description: t("userDetail.toast.passwordResetSuccess", { username: user.username }),
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userDetail.toast.passwordResetError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteUser = async () => {
    if (!user) return;
    if (!window.confirm(t("userDetail.dialog.confirmDelete", { username: user.username }))) {
      return;
    }
    try {
      await apiFetch(`/api/v1/admin/users/${user.id}`, { method: "DELETE" });
      toast({
        title: t("common.appName"),
        description: t("userDetail.toast.deleteSuccess", { username: user.username }),
      });
      window.location.href = "/users";
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userDetail.toast.deleteError"),
        variant: "destructive",
      });
    }
  };

  if (isLoading || !user) {
    return (
      <div className="h-64 flex items-center justify-center space-x-2">
        <RefreshCw className="h-5 w-5 animate-spin text-primary" />
        <span className="text-sm text-muted-foreground">{t("common.loading")}</span>
      </div>
    );
  }

  const diskPercent =
    user.resources.disk_limit_mb > 0
      ? Math.min(100, Math.round((user.resources.disk_used_mb / user.resources.disk_limit_mb) * 100))
      : 0;

  const memPercent =
    user.resources.memory_limit_mb > 0
      ? Math.min(100, Math.round((user.resources.memory_used_mb / user.resources.memory_limit_mb) * 100))
      : 0;

  const tabs = [
    { id: "overview" as UserDetailTab, label: t("userDetail.tabs.overview"), icon: Layers },
    { id: "resources" as UserDetailTab, label: t("userDetail.tabs.resources"), icon: Cpu },
    { id: "domains" as UserDetailTab, label: t("userDetail.tabs.domains"), icon: Globe },
    { id: "databases" as UserDetailTab, label: t("userDetail.tabs.databases"), icon: Database },
    { id: "logs" as UserDetailTab, label: t("userDetail.tabs.logs"), icon: FileText },
    { id: "activity" as UserDetailTab, label: t("userDetail.tabs.activity"), icon: Activity },
  ];

  return (
    <div className="space-y-6 max-w-6xl mx-auto pb-12">
      {/* Back button & Title */}
      <div className="flex items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => (window.location.href = "/users")}
          className="gap-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("userDetail.backToUsers")}
        </Button>
      </div>

      {/* User Header Profile Card */}
      <Card className="border shadow-sm bg-card overflow-hidden">
        <CardContent className="p-6">
          <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
            {/* Identity & Status */}
            <div className="flex items-start gap-4">
              <div className="h-16 w-16 rounded-2xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary font-bold text-xl uppercase shadow-sm shrink-0">
                {user.username.slice(0, 2)}
              </div>
              <div className="space-y-1">
                <div className="flex items-center gap-3">
                  <h1 className="text-2xl font-bold tracking-tight text-foreground">
                    {user.username}
                  </h1>
                  {user.status === "active" ? (
                    <Badge variant="default" className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
                      {t("userDetail.status.active")}
                    </Badge>
                  ) : user.status === "suspended" ? (
                    <Badge variant="destructive">{t("userDetail.status.suspended")}</Badge>
                  ) : (
                    <Badge variant="secondary">{t("userDetail.status.pending")}</Badge>
                  )}
                  <Badge variant="outline" className="font-mono text-xs">
                    {user.plan_name}
                  </Badge>
                </div>

                <p className="text-xs text-muted-foreground flex items-center gap-2">
                  <span>{user.email}</span>
                  <span>•</span>
                  <span className="font-mono">UID: {user.uid}</span>
                  <span>•</span>
                  <span className="font-mono">{user.node_name}</span>
                </p>

                {user.primary_domain && (
                  <p className="text-xs font-mono text-primary flex items-center gap-1 pt-0.5">
                    <Globe className="h-3.5 w-3.5" />
                    <a
                      href={`http://${user.primary_domain}`}
                      target="_blank"
                      rel="noreferrer"
                      className="hover:underline flex items-center gap-1"
                    >
                      {user.primary_domain}
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </p>
                )}
              </div>
            </div>

            {/* Action Buttons Toolbar */}
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={handleImpersonate} className="gap-2 text-xs">
                <LogIn className="h-4 w-4" />
                {t("userDetail.actions.impersonate")}
              </Button>

              <Button
                size="sm"
                variant={user.status === "suspended" ? "default" : "outline"}
                onClick={handleToggleSuspend}
                className={`gap-1.5 text-xs ${
                  user.status === "suspended"
                    ? "bg-emerald-600 hover:bg-emerald-700 text-white"
                    : "text-destructive hover:bg-destructive/10 border-destructive/30"
                }`}
              >
                {user.status === "suspended" ? (
                  <>
                    <ShieldCheck className="h-4 w-4" />
                    {t("userDetail.actions.unsuspend")}
                  </>
                ) : (
                  <>
                    <ShieldAlert className="h-4 w-4" />
                    {t("userDetail.actions.suspend")}
                  </>
                )}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => setIsResetPassOpen(true)}
                className="gap-1.5 text-xs"
              >
                <KeyRound className="h-4 w-4" />
                {t("userDetail.actions.resetPassword")}
              </Button>

              <Button
                size="sm"
                variant="ghost"
                onClick={handleDeleteUser}
                className="h-9 w-9 p-0 text-muted-foreground hover:text-destructive"
                title={t("userDetail.actions.delete")}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Tab Navigation */}
      <div className="border-b flex items-center gap-1 overflow-x-auto">
        {tabs.map((tab) => {
          const Icon = tab.icon;
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium border-b-2 transition-all whitespace-nowrap ${
                isActive
                  ? "border-primary text-primary"
                  : "border-transparent text-muted-foreground hover:text-foreground hover:border-muted-foreground/30"
              }`}
            >
              <Icon className="h-4 w-4" />
              <span>{tab.label}</span>
            </button>
          );
        })}
      </div>

      {/* Tab 1: Overview */}
      {activeTab === "overview" && (
        <div className="space-y-6">
          {/* Quick Metrics Cards */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            <Card className="p-4 border bg-card">
              <span className="text-xs text-muted-foreground block">CPU Limit</span>
              <span className="text-xl font-bold font-mono text-foreground mt-1 block">
                {user.resources.cpu_cores === -1 ? "∞ Unlimited" : `${user.resources.cpu_cores} Cores`}
              </span>
              <span className="text-[11px] text-muted-foreground mt-0.5 block">
                Load: {user.resources.cpu_used_pct}%
              </span>
            </Card>
            <Card className="p-4 border bg-card">
              <span className="text-xs text-muted-foreground block">Memory RAM</span>
              <span className="text-xl font-bold font-mono text-foreground mt-1 block">
                {formatBytes(user.resources.memory_used_mb)}
              </span>
              <span className="text-[11px] text-muted-foreground mt-0.5 block">
                Quota: {formatBytes(user.resources.memory_limit_mb)} ({memPercent}%)
              </span>
            </Card>
            <Card className="p-4 border bg-card">
              <span className="text-xs text-muted-foreground block">Disk Usage</span>
              <span className="text-xl font-bold font-mono text-foreground mt-1 block">
                {formatBytes(user.resources.disk_used_mb)}
              </span>
              <span className="text-[11px] text-muted-foreground mt-0.5 block">
                Limit: {formatBytes(user.resources.disk_limit_mb)} ({diskPercent}%)
              </span>
            </Card>
            <Card className="p-4 border bg-card">
              <span className="text-xs text-muted-foreground block">Container Stack</span>
              <div className="flex items-center gap-2 mt-1">
                <span className="relative flex h-2.5 w-2.5">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
                  <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500" />
                </span>
                <span className="font-bold text-emerald-600 dark:text-emerald-400 font-mono text-sm capitalize">
                  {user.container.status}
                </span>
              </div>
              <span className="text-[11px] text-muted-foreground mt-0.5 block truncate">
                {user.container.uptime}
              </span>
            </Card>
          </div>

          {/* Identity & Stack Details */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <Card className="border bg-card">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <User className="h-4 w-4 text-primary" />
                  <span>{t("userDetail.overview.accountDetails")}</span>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-xs">
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Linux UID</span>
                  <span className="font-mono font-semibold">{user.uid}</span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Filesystem Root</span>
                  <span className="font-mono font-semibold">{user.root_path}</span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Assigned Node</span>
                  <span className="font-mono font-semibold">{user.node_name}</span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Registration Date</span>
                  <span className="font-mono">{new Date(user.created_at).toLocaleDateString()}</span>
                </div>
                {user.suspended_at && (
                  <div className="flex justify-between py-1.5 border-b text-destructive">
                    <span>Suspended Reason</span>
                    <span className="font-semibold">{user.suspended_reason || "Admin review"}</span>
                  </div>
                )}
              </CardContent>
            </Card>

            <Card className="border bg-card">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <Box className="h-4 w-4 text-emerald-500" />
                  <span>{t("userDetail.overview.containerStack")}</span>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-xs">
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Container Name</span>
                  <span className="font-mono font-semibold">{user.container.name}</span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Stack Image</span>
                  <span className="font-mono font-semibold truncate max-w-[240px]">
                    {user.container.image}
                  </span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Bridge IP</span>
                  <span className="font-mono font-semibold">{user.container.ip_address}</span>
                </div>
                <div className="flex justify-between py-1.5 border-b">
                  <span className="text-muted-foreground">Security Sandbox</span>
                  <span className="font-semibold text-emerald-600 dark:text-emerald-400">
                    Rootless / Cap-Drop ALL
                  </span>
                </div>
              </CardContent>
            </Card>
          </div>
        </div>
      )}

      {/* Tab 2: Resources */}
      {activeTab === "resources" && (
        <div className="space-y-4">
          <Card className="border bg-card">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Cpu className="h-4 w-4 text-primary" />
                <span>{t("userDetail.resources.title")}</span>
              </CardTitle>
              <CardDescription>{t("userDetail.resources.desc")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6 pt-2">
              {/* CPU */}
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs">
                  <span className="font-medium text-foreground">CPU Cores & Frequency</span>
                  <span className="font-mono text-muted-foreground">
                    {user.resources.cpu_cores === -1 ? "Unlimited" : `${user.resources.cpu_cores} Cores`}{" "}
                    ({user.resources.cpu_used_pct}% load)
                  </span>
                </div>
                <div className="w-full h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-primary rounded-full transition-all"
                    style={{ width: `${user.resources.cpu_used_pct}%` }}
                  />
                </div>
              </div>

              {/* Memory RAM */}
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs">
                  <span className="font-medium text-foreground">Memory (RAM)</span>
                  <span className="font-mono text-muted-foreground">
                    {formatBytes(user.resources.memory_used_mb)} / {formatBytes(user.resources.memory_limit_mb)} ({memPercent}%)
                  </span>
                </div>
                <div className="w-full h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-blue-500 rounded-full transition-all"
                    style={{ width: `${memPercent}%` }}
                  />
                </div>
              </div>

              {/* Disk Storage */}
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs">
                  <span className="font-medium text-foreground">Filesystem Storage (XFS Quota)</span>
                  <span className="font-mono text-muted-foreground">
                    {formatBytes(user.resources.disk_used_mb)} / {formatBytes(user.resources.disk_limit_mb)} ({diskPercent}%)
                  </span>
                </div>
                <div className="w-full h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-emerald-500 rounded-full transition-all"
                    style={{ width: `${diskPercent}%` }}
                  />
                </div>
              </div>

              {/* Inodes */}
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs">
                  <span className="font-medium text-foreground">Inodes (File Count)</span>
                  <span className="font-mono text-muted-foreground">
                    {user.resources.inodes_used.toLocaleString()} /{" "}
                    {user.resources.inodes_limit === -1
                      ? "∞ Unlimited"
                      : user.resources.inodes_limit.toLocaleString()}
                  </span>
                </div>
                <div className="w-full h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-amber-500 rounded-full transition-all"
                    style={{
                      width: `${
                        user.resources.inodes_limit > 0
                          ? Math.round(
                              (user.resources.inodes_used / user.resources.inodes_limit) * 100
                            )
                          : 0
                      }%`,
                    }}
                  />
                </div>
              </div>

              {/* PIDs Limit */}
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs">
                  <span className="font-medium text-foreground">Process Limit (PIDs)</span>
                  <span className="font-mono text-muted-foreground">
                    {user.resources.pids_used} /{" "}
                    {user.resources.pids_limit === -1 ? "∞ Unlimited" : user.resources.pids_limit}
                  </span>
                </div>
                <div className="w-full h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-purple-500 rounded-full transition-all"
                    style={{
                      width: `${
                        user.resources.pids_limit > 0
                          ? Math.round((user.resources.pids_used / user.resources.pids_limit) * 100)
                          : 0
                      }%`,
                    }}
                  />
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Tab 3: Domains */}
      {activeTab === "domains" && (
        <Card className="border bg-card">
          <CardHeader className="pb-3 flex flex-row items-center justify-between">
            <div>
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Globe className="h-4 w-4 text-primary" />
                <span>{t("userDetail.domains.title")}</span>
              </CardTitle>
              <CardDescription>{t("userDetail.domains.desc")}</CardDescription>
            </div>
            <Badge variant="outline" className="text-xs">
              {user.domains.length} Domains
            </Badge>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader className="bg-muted/50">
                <TableRow>
                  <TableHead className="w-[30%]">Domain</TableHead>
                  <TableHead className="w-[35%]">Document Root</TableHead>
                  <TableHead className="w-[15%]">SSL Status</TableHead>
                  <TableHead className="w-[20%]">PHP Runtime</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {user.domains.map((dom) => (
                  <TableRow key={dom.id}>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-foreground font-mono text-xs">
                          {dom.domain}
                        </span>
                        {dom.is_primary && (
                          <Badge variant="secondary" className="text-[10px]">
                            Primary
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {dom.docroot}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant="default"
                        className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30 text-[10px]"
                      >
                        Let&apos;s Encrypt
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs">{dom.php_version}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* Tab 4: Databases */}
      {activeTab === "databases" && (
        <Card className="border bg-card">
          <CardHeader className="pb-3 flex flex-row items-center justify-between">
            <div>
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Database className="h-4 w-4 text-primary" />
                <span>{t("userDetail.databases.title")}</span>
              </CardTitle>
              <CardDescription>{t("userDetail.databases.desc")}</CardDescription>
            </div>
            <Badge variant="outline" className="text-xs">
              Phase 07 Preview
            </Badge>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader className="bg-muted/50">
                <TableRow>
                  <TableHead className="w-[35%]">Database Name</TableHead>
                  <TableHead className="w-[25%]">Database Engine</TableHead>
                  <TableHead className="w-[20%]">Size</TableHead>
                  <TableHead className="w-[20%]">Charset</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {user.databases.map((db) => (
                  <TableRow key={db.id}>
                    <TableCell className="font-mono text-xs font-semibold">{db.name}</TableCell>
                    <TableCell className="capitalize text-xs font-mono">{db.type}</TableCell>
                    <TableCell className="text-xs font-mono">{db.size_mb} MB</TableCell>
                    <TableCell className="text-xs font-mono text-muted-foreground">
                      {db.charset}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* Tab 5: Logs */}
      {activeTab === "logs" && (
        <Card className="border bg-card">
          <CardHeader className="pb-3 flex flex-row items-center justify-between">
            <div>
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Terminal className="h-4 w-4 text-primary" />
                <span>{t("userDetail.logs.title")}</span>
              </CardTitle>
              <CardDescription>{t("userDetail.logs.desc")}</CardDescription>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={fetchUserDetail}
              className="gap-1.5 text-xs h-8"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              Refresh
            </Button>
          </CardHeader>
          <CardContent className="p-4">
            <div className="rounded-lg bg-zinc-950 text-zinc-100 p-4 font-mono text-xs space-y-2 max-h-96 overflow-y-auto border border-zinc-800">
              {user.logs.map((log) => (
                <div key={log.id} className="flex items-start gap-2.5 leading-relaxed">
                  <span className="text-zinc-500 shrink-0 text-[11px]">
                    {new Date(log.timestamp).toLocaleTimeString()}
                  </span>
                  <span
                    className={`px-1 rounded text-[10px] font-bold uppercase shrink-0 ${
                      log.level === "error"
                        ? "bg-red-950 text-red-400"
                        : log.level === "warn"
                        ? "bg-amber-950 text-amber-400"
                        : "bg-blue-950 text-blue-400"
                    }`}
                  >
                    {log.source}
                  </span>
                  <span className="text-zinc-300 break-all">{log.message}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Tab 6: Activity */}
      {activeTab === "activity" && (
        <Card className="border bg-card">
          <CardHeader className="pb-3">
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <Activity className="h-4 w-4 text-primary" />
              <span>{t("userDetail.activity.title")}</span>
            </CardTitle>
            <CardDescription>{t("userDetail.activity.desc")}</CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader className="bg-muted/50">
                <TableRow>
                  <TableHead className="w-[20%]">Timestamp</TableHead>
                  <TableHead className="w-[25%]">Action</TableHead>
                  <TableHead className="w-[20%]">Actor</TableHead>
                  <TableHead className="w-[15%]">Status</TableHead>
                  <TableHead className="w-[20%]">Details</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {user.activities.map((act) => (
                  <TableRow key={act.id}>
                    <TableCell className="text-xs text-muted-foreground font-mono">
                      {new Date(act.timestamp).toLocaleString()}
                    </TableCell>
                    <TableCell className="font-mono text-xs font-semibold">{act.action}</TableCell>
                    <TableCell className="text-xs">{act.actor}</TableCell>
                    <TableCell>
                      <Badge
                        variant="default"
                        className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30 text-[10px]"
                      >
                        {act.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">{act.details}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* Reset Password Dialog */}
      <ResetPasswordDialog
        open={isResetPassOpen}
        onOpenChange={setIsResetPassOpen}
        username={user.username}
        onConfirm={handleResetPassword}
        isSubmitting={isSubmitting}
      />
    </div>
  );
}
