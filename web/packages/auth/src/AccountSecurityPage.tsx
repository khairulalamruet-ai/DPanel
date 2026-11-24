import React, { useEffect, useState } from "react";
import {
  Button,
  Input,
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import { api, getCsrfToken, apiFetch } from "@builtpanel/api";
import {
  ShieldCheck,
  ShieldAlert,
  KeyRound,
  Laptop,
  Smartphone,
  Globe,
  History,
  Copy,
  Check,
  RefreshCw,
  Trash2,
  Lock,
  AlertCircle,
  CheckCircle2,
} from "lucide-react";

export function AccountSecurityPage() {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();

  const [activeTab, setActiveTab] = useState<"password" | "mfa" | "sessions" | "history">("password");

  // -------------------------------------------------------------
  // 1. Password Change State
  // -------------------------------------------------------------
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [passwordLoading, setPasswordLoading] = useState(false);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [passwordSuccess, setPasswordSuccess] = useState<string | null>(null);

  const handlePasswordChange = async (e: React.FormEvent) => {
    e.preventDefault();
    setPasswordError(null);
    setPasswordSuccess(null);

    if (!currentPassword) {
      setPasswordError(t("security.password.currentPlaceholder"));
      return;
    }
    if (newPassword.length < 12) {
      setPasswordError(t("security.password.tooShort"));
      return;
    }
    if (newPassword !== confirmPassword) {
      setPasswordError(t("security.password.mismatch"));
      return;
    }

    try {
      setPasswordLoading(true);
      await apiFetch("/api/v1/auth/password", {
        method: "PUT",
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      });

      setPasswordSuccess(t("security.password.success"));
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
    } catch (err: any) {
      // A refused new password comes with what is wrong with it, field by field.
      const fields: { message: string }[] = err?.errors ?? [];
      setPasswordError(fields.length ? fields.map((f) => f.message).join(" ") : err.message || t("common.error"));
    } finally {
      setPasswordLoading(false);
    }
  };

  // -------------------------------------------------------------
  // 2. 2FA (MFA) State
  // -------------------------------------------------------------
  const [mfaEnabled, setMfaEnabled] = useState(false);
  const [mfaLoading, setMfaLoading] = useState(true);
  const [enrollData, setEnrollData] = useState<{ secret: string; otpauth_uri: string } | null>(null);
  const [totpInput, setTotpInput] = useState("");
  const [totpError, setTotpError] = useState<string | null>(null);
  const [backupCodes, setBackupCodes] = useState<string[] | null>(null);
  const [copiedCodes, setCopiedCodes] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);

  // Disable 2FA Modal
  const [showDisableModal, setShowDisableModal] = useState(false);
  const [disablePassword, setDisablePassword] = useState("");
  const [disableError, setDisableError] = useState<string | null>(null);
  const [disableLoading, setDisableLoading] = useState(false);

  const fetchMfaStatus = async () => {
    try {
      setMfaLoading(true);
      const res = await api.GET("/api/v1/auth/mfa");
      if (res.data) {
        setMfaEnabled(res.data.totp_enabled);
      }
    } catch {
      // Ignore initial status fetch failure in tests
    } finally {
      setMfaLoading(false);
    }
  };

  const handleStartEnroll = async () => {
    try {
      setMfaLoading(true);
      setTotpError(null);
      const res = await api.POST("/api/v1/auth/mfa/totp/enroll", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
      if (res.data) {
        setEnrollData(res.data);
      }
    } catch {
      setTotpError(t("common.error"));
    } finally {
      setMfaLoading(false);
    }
  };

  const handleConfirmTotp = async (e: React.FormEvent) => {
    e.preventDefault();
    setTotpError(null);
    if (!totpInput || totpInput.length < 6) {
      setTotpError(t("security.mfa.invalidCode"));
      return;
    }

    try {
      setMfaLoading(true);
      const res = await api.POST("/api/v1/auth/mfa/totp/confirm", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
        body: { code: totpInput.trim() },
      });

      if (res.error) {
        setTotpError(t("security.mfa.invalidCode"));
        return;
      }

      setMfaEnabled(true);
      setEnrollData(null);
      setTotpInput("");
      if (res.data?.backup_codes) {
        setBackupCodes(res.data.backup_codes);
      }
    } catch {
      setTotpError(t("common.error"));
    } finally {
      setMfaLoading(false);
    }
  };

  const handleRegenerateBackupCodes = async () => {
    try {
      setMfaLoading(true);
      // Enrolling anew or getting backup codes
      const res = await api.POST("/api/v1/auth/mfa/totp/enroll", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
      if (res.data) {
        setEnrollData(res.data);
      }
    } catch {
      // Ignore
    } finally {
      setMfaLoading(false);
    }
  };

  const handleDisableMfa = async (e: React.FormEvent) => {
    e.preventDefault();
    setDisableError(null);
    if (!disablePassword) {
      setDisableError(t("security.password.currentPlaceholder"));
      return;
    }

    try {
      setDisableLoading(true);
      // Sudo check first
      await api.POST("/api/v1/auth/sudo", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
        body: { password: disablePassword },
      });

      const res = await api.POST("/api/v1/auth/mfa/totp/disable", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
      if (res.error) {
        setDisableError(t("auth.invalidCredentials"));
        return;
      }

      setMfaEnabled(false);
      setShowDisableModal(false);
      setDisablePassword("");
      setBackupCodes(null);
    } catch {
      setDisableError(t("common.error"));
    } finally {
      setDisableLoading(false);
    }
  };

  const copyToClipboard = (text: string, type: "secret" | "codes") => {
    navigator.clipboard?.writeText(text);
    if (type === "secret") {
      setCopiedSecret(true);
      setTimeout(() => setCopiedSecret(false), 2000);
    } else {
      setCopiedCodes(true);
      setTimeout(() => setCopiedCodes(false), 2000);
    }
  };

  // -------------------------------------------------------------
  // 3. Active Sessions State
  // -------------------------------------------------------------
  interface SessionItem {
    id: string;
    ip?: string | null;
    user_agent?: string | null;
    created_at: string;
    last_seen_at?: string;
    current: boolean;
  }
  const [sessions, setSessions] = useState<SessionItem[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(false);
  const [sessionsMessage, setSessionsMessage] = useState<string | null>(null);

  const fetchSessions = async () => {
    try {
      setSessionsLoading(true);
      const res = await api.GET("/api/v1/auth/sessions");
      if (res.data?.sessions) {
        setSessions(res.data.sessions as any);
      }
    } catch {
      // Ignore
    } finally {
      setSessionsLoading(false);
    }
  };

  const handleRevokeSession = async (id: string) => {
    try {
      const res = await api.DELETE("/api/v1/auth/sessions/{id}", {
        params: { path: { id }, header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
      if (res.error) {
        return;
      }
      setSessionsMessage(t("security.sessions.revokedSuccess"));
      await fetchSessions();
    } catch {
      // Ignore
    }
  };

  const handleRevokeAllOthers = async () => {
    try {
      const res = await api.POST("/api/v1/auth/sessions/revoke-others", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
      if (res.error) {
        return;
      }
      setSessionsMessage(t("security.sessions.revokeOthersSuccess"));
      await fetchSessions();
    } catch {
      // Ignore
    }
  };

  // -------------------------------------------------------------
  // 4. Login History State
  // -------------------------------------------------------------
  interface LoginEventItem {
    id: string;
    ip_address: string;
    user_agent: string;
    status: "success" | "failed" | "2fa_prompt";
    created_at: string;
  }
  const [loginHistory, setLoginHistory] = useState<LoginEventItem[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);

  const fetchHistory = async () => {
    try {
      setHistoryLoading(true);
      // The sign-ins of this account, from what the panel wrote down: a customer's own activity
      // feed, or the audit log for an administrator.
      const me = await apiFetch<{ id: string; kind: string }>("/api/v1/auth/me");
      const events =
        me.kind === "admin"
          ? (await apiFetch<{ items: { id: string; action: string; at: string; actor_ip?: string | null; outcome: string }[] }>(
              `/api/v1/admin/audit-logs?actor_id=${encodeURIComponent(me.id)}&limit=100`,
            )).items.map((e) => ({ id: e.id, action: e.action, at: e.at, ip: e.actor_ip, outcome: e.outcome }))
          : (await apiFetch<{ items: { id: string; action: string; at: string; ip?: string | null; outcome: string }[] }>(
              "/api/v1/client/activity?limit=100",
            )).items;
      setLoginHistory(
        events
          .filter((e) => e.action.startsWith("auth.login"))
          .map((e) => ({
            id: e.id,
            ip_address: e.ip ?? "",
            user_agent: "",
            status: e.outcome === "success" ? ("success" as const) : ("failed" as const),
            created_at: e.at,
          })),
      );
    } catch {
      // Ignore
    } finally {
      setHistoryLoading(false);
    }
  };

  useEffect(() => {
    fetchMfaStatus();
    fetchSessions();
    fetchHistory();
  }, []);

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t("security.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("security.desc")}</p>
      </div>

      {/* Tabs navigation */}
      <div className="flex border-b border-border space-x-2" role="tablist">
        <button
          role="tab"
          aria-selected={activeTab === "password"}
          onClick={() => setActiveTab("password")}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "password"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("security.tabs.password")}
        </button>
        <button
          role="tab"
          aria-selected={activeTab === "mfa"}
          onClick={() => setActiveTab("mfa")}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors flex items-center gap-1.5 ${
            activeTab === "mfa"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("security.tabs.mfa")}
          {mfaEnabled ? (
            <Badge variant="success" className="text-[10px] py-0 px-1.5">
              ON
            </Badge>
          ) : (
            <Badge variant="secondary" className="text-[10px] py-0 px-1.5">
              OFF
            </Badge>
          )}
        </button>
        <button
          role="tab"
          aria-selected={activeTab === "sessions"}
          onClick={() => {
            setActiveTab("sessions");
            fetchSessions();
          }}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "sessions"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("security.tabs.sessions")}
        </button>
        <button
          role="tab"
          aria-selected={activeTab === "history"}
          onClick={() => {
            setActiveTab("history");
            fetchHistory();
          }}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "history"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("security.tabs.history")}
        </button>
      </div>

      {/* TAB 1: PASSWORD CHANGE */}
      {activeTab === "password" && (
        <Card className="max-w-xl">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <KeyRound className="h-5 w-5 text-primary" />
              {t("security.password.changeTitle")}
            </CardTitle>
            <CardDescription>{t("security.password.desc")}</CardDescription>
          </CardHeader>
          <form onSubmit={handlePasswordChange} noValidate>
            <CardContent className="space-y-4">
              {passwordError && (
                <div role="alert" className="p-3 text-sm rounded bg-destructive/15 text-destructive flex items-center gap-2">
                  <AlertCircle className="h-4 w-4 shrink-0" />
                  <span>{passwordError}</span>
                </div>
              )}
              {passwordSuccess && (
                <div role="status" className="p-3 text-sm rounded bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 flex items-center gap-2">
                  <CheckCircle2 className="h-4 w-4 shrink-0" />
                  <span>{passwordSuccess}</span>
                </div>
              )}

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="currentPassword">
                  {t("security.password.current")}
                </label>
                <Input
                  id="currentPassword"
                  name="currentPassword"
                  type="password"
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  placeholder={t("security.password.currentPlaceholder")}
                  disabled={passwordLoading}
                  required
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="newPassword">
                  {t("security.password.new")}
                </label>
                <Input
                  id="newPassword"
                  name="newPassword"
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder={t("security.password.newPlaceholder")}
                  disabled={passwordLoading}
                  required
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-semibold uppercase text-muted-foreground" htmlFor="confirmPassword">
                  {t("security.password.confirm")}
                </label>
                <Input
                  id="confirmPassword"
                  name="confirmPassword"
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  placeholder={t("security.password.confirmPlaceholder")}
                  disabled={passwordLoading}
                  required
                />
              </div>
            </CardContent>
            <CardFooter>
              <Button type="submit" disabled={passwordLoading}>
                {passwordLoading ? t("security.password.updating") : t("security.password.updateBtn")}
              </Button>
            </CardFooter>
          </form>
        </Card>
      )}

      {/* TAB 2: TWO-FACTOR AUTHENTICATION */}
      {activeTab === "mfa" && (
        <div className="space-y-6 max-w-2xl">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0">
              <div>
                <CardTitle className="flex items-center gap-2">
                  {mfaEnabled ? (
                    <ShieldCheck className="h-5 w-5 text-emerald-500" />
                  ) : (
                    <ShieldAlert className="h-5 w-5 text-amber-500" />
                  )}
                  {t("security.mfa.title")}
                </CardTitle>
                <CardDescription className="mt-1">{t("security.mfa.desc")}</CardDescription>
              </div>
              <Badge variant={mfaEnabled ? "success" : "secondary"} className="text-xs px-2.5 py-1">
                {mfaEnabled ? t("security.mfa.enabled") : t("security.mfa.disabled")}
              </Badge>
            </CardHeader>

            <CardContent className="space-y-4 pt-4">
              {!mfaEnabled && !enrollData && (
                <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 p-4 rounded-lg bg-muted/40 border border-muted">
                  <div className="space-y-1">
                    <p className="font-medium text-sm">{t("security.mfa.enableBtn")}</p>
                    <p className="text-xs text-muted-foreground">{t("security.mfa.desc")}</p>
                  </div>
                  <Button onClick={handleStartEnroll} disabled={mfaLoading}>
                    {t("security.mfa.enableBtn")}
                  </Button>
                </div>
              )}

              {/* Enrollment Step */}
              {!mfaEnabled && enrollData && (
                <div className="space-y-6 border rounded-lg p-5 bg-card">
                  <p className="text-sm text-foreground font-medium">{t("security.mfa.scanQr")}</p>

                  <div className="flex flex-col md:flex-row gap-6 items-center">
                    {/* Visual QR Code Pattern */}
                    <div className="p-3 bg-white rounded-lg border shadow-sm w-44 h-44 flex flex-col items-center justify-center">
                      <svg viewBox="0 0 100 100" className="w-36 h-36" fill="black">
                        <rect x="5" y="5" width="25" height="25" fill="black" />
                        <rect x="10" y="10" width="15" height="15" fill="white" />
                        <rect x="13" y="13" width="9" height="9" fill="black" />

                        <rect x="70" y="5" width="25" height="25" fill="black" />
                        <rect x="75" y="10" width="15" height="15" fill="white" />
                        <rect x="78" y="13" width="9" height="9" fill="black" />

                        <rect x="5" y="70" width="25" height="25" fill="black" />
                        <rect x="10" y="75" width="15" height="15" fill="white" />
                        <rect x="13" y="78" width="9" height="9" fill="black" />

                        <rect x="40" y="20" width="20" height="10" fill="black" />
                        <rect x="45" y="45" width="10" height="10" fill="black" />
                        <rect x="65" y="40" width="10" height="20" fill="black" />
                        <rect x="40" y="70" width="20" height="10" fill="black" />
                        <rect x="70" y="75" width="20" height="15" fill="black" />
                      </svg>
                    </div>

                    <div className="space-y-4 flex-1 w-full">
                      <div className="space-y-1">
                        <label className="text-xs font-semibold uppercase text-muted-foreground">
                          {t("security.mfa.secretKey")}
                        </label>
                        <div className="flex items-center gap-2">
                          <code className="px-3 py-1.5 rounded bg-muted font-mono text-sm tracking-wider flex-1 select-all">
                            {enrollData.secret}
                          </code>
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={() => copyToClipboard(enrollData.secret, "secret")}
                            className="gap-1 text-xs"
                          >
                            {copiedSecret ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                            {copiedSecret ? t("security.mfa.copied") : t("security.mfa.copyAll")}
                          </Button>
                        </div>
                      </div>

                      <form onSubmit={handleConfirmTotp} className="space-y-3" noValidate>
                        {totpError && (
                          <div role="alert" className="p-2 text-xs rounded bg-destructive/15 text-destructive flex items-center gap-1.5">
                            <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                            <span>{totpError}</span>
                          </div>
                        )}
                        <div className="space-y-1">
                          <label htmlFor="totpInput" className="text-xs font-semibold uppercase text-muted-foreground">
                            {t("security.mfa.enterCode")}
                          </label>
                          <Input
                            id="totpInput"
                            name="totpInput"
                            type="text"
                            inputMode="numeric"
                            maxLength={6}
                            placeholder="123456"
                            value={totpInput}
                            onChange={(e) => setTotpInput(e.target.value.replace(/\D/g, ""))}
                            disabled={mfaLoading}
                            required
                          />
                        </div>
                        <div className="flex gap-2">
                          <Button type="submit" disabled={mfaLoading || totpInput.length < 6}>
                            {mfaLoading ? t("security.mfa.verifying") : t("security.mfa.verifyAndEnable")}
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            onClick={() => {
                              setEnrollData(null);
                              setTotpInput("");
                              setTotpError(null);
                            }}
                          >
                            {t("security.mfa.cancel")}
                          </Button>
                        </div>
                      </form>
                    </div>
                  </div>
                </div>
              )}

              {/* Enabled State Actions */}
              {mfaEnabled && (
                <div className="space-y-4">
                  <div className="flex flex-col sm:flex-row gap-3 pt-2">
                    <Button variant="outline" onClick={handleRegenerateBackupCodes} disabled={mfaLoading} className="gap-2">
                      <RefreshCw className="h-4 w-4" />
                      {t("security.mfa.regenerateCodes")}
                    </Button>
                    <Button
                      variant="destructive"
                      onClick={() => setShowDisableModal(true)}
                      disabled={mfaLoading}
                      className="gap-2"
                    >
                      <Trash2 className="h-4 w-4" />
                      {t("security.mfa.disableBtn")}
                    </Button>
                  </div>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Backup Codes Display */}
          {backupCodes && backupCodes.length > 0 && (
            <Card className="border-primary/40 bg-primary/5">
              <CardHeader>
                <div className="flex items-center justify-between">
                  <CardTitle className="text-base flex items-center gap-2">
                    <Lock className="h-4 w-4 text-primary" />
                    {t("security.mfa.backupCodes")}
                  </CardTitle>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => copyToClipboard(backupCodes.join("\n"), "codes")}
                    className="gap-1 text-xs"
                  >
                    {copiedCodes ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                    {copiedCodes ? t("security.mfa.copied") : t("security.mfa.copyAll")}
                  </Button>
                </div>
                <CardDescription className="text-xs">{t("security.mfa.backupCodesDesc")}</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="grid grid-cols-2 sm:grid-cols-5 gap-2">
                  {backupCodes.map((code, index) => (
                    <div
                      key={index}
                      className="p-2 text-center rounded bg-background border font-mono text-xs tracking-wider select-all"
                    >
                      {code}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          )}

          {/* Disable 2FA Modal */}
          <Dialog open={showDisableModal} onOpenChange={setShowDisableModal}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t("security.mfa.disableConfirmTitle")}</DialogTitle>
                <DialogDescription>{t("security.mfa.disableConfirmDesc")}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleDisableMfa} className="space-y-4" noValidate>
                {disableError && (
                  <div role="alert" className="p-2.5 text-xs rounded bg-destructive/15 text-destructive flex items-center gap-2">
                    <AlertCircle className="h-4 w-4 shrink-0" />
                    <span>{disableError}</span>
                  </div>
                )}
                <div className="space-y-1.5">
                  <label htmlFor="disablePassword" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("security.password.current")}
                  </label>
                  <Input
                    id="disablePassword"
                    name="disablePassword"
                    type="password"
                    placeholder={t("security.password.currentPlaceholder")}
                    value={disablePassword}
                    onChange={(e) => setDisablePassword(e.target.value)}
                    disabled={disableLoading}
                    required
                  />
                </div>
                <DialogFooter className="gap-2 sm:gap-0">
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() => {
                      setShowDisableModal(false);
                      setDisablePassword("");
                      setDisableError(null);
                    }}
                  >
                    {t("security.mfa.cancel")}
                  </Button>
                  <Button type="submit" variant="destructive" disabled={disableLoading || !disablePassword}>
                    {disableLoading ? t("security.password.updating") : t("security.mfa.disableConfirmBtn")}
                  </Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>
      )}

      {/* TAB 3: ACTIVE SESSIONS */}
      {activeTab === "sessions" && (
        <Card>
          <CardHeader className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
            <div>
              <CardTitle className="flex items-center gap-2">
                <Laptop className="h-5 w-5 text-primary" />
                {t("security.sessions.title")}
              </CardTitle>
              <CardDescription>{t("security.sessions.desc")}</CardDescription>
            </div>
            {sessions.some((s) => !s.current) && (
              <Button variant="outline" size="sm" onClick={handleRevokeAllOthers} className="gap-2 self-start sm:self-auto">
                <Trash2 className="h-4 w-4 text-destructive" />
                {t("security.sessions.revokeOthers")}
              </Button>
            )}
          </CardHeader>
          <CardContent className="space-y-4">
            {sessionsMessage && (
              <div role="status" className="p-3 text-sm rounded bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 flex items-center gap-2">
                <CheckCircle2 className="h-4 w-4 shrink-0" />
                <span>{sessionsMessage}</span>
              </div>
            )}

            <div className="rounded-md border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("security.sessions.device")}</TableHead>
                    <TableHead>{t("security.sessions.ip")}</TableHead>
                    <TableHead>{t("security.sessions.lastActive")}</TableHead>
                    <TableHead className="text-right"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sessionsLoading ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-center py-6 text-muted-foreground text-sm">
                        {t("common.loading")}
                      </TableCell>
                    </TableRow>
                  ) : sessions.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-center py-6 text-muted-foreground text-sm">
                        No sessions found
                      </TableCell>
                    </TableRow>
                  ) : (
                    sessions.map((session) => {
                      const ua = session.user_agent || "Browser Session";
                      const ip = session.ip || "127.0.0.1";
                      const activeTime = session.last_seen_at || session.created_at;
                      return (
                        <TableRow key={session.id}>
                          <TableCell>
                            <div className="flex items-center gap-2">
                              {ua.toLowerCase().includes("mobile") ||
                              ua.toLowerCase().includes("iphone") ? (
                                <Smartphone className="h-4 w-4 text-muted-foreground" />
                              ) : (
                                <Laptop className="h-4 w-4 text-muted-foreground" />
                              )}
                              <div className="flex flex-col">
                                <span className="font-medium text-sm flex items-center gap-1.5">
                                  {ua.split(") ")[1] || ua.slice(0, 30)}
                                  {session.current && (
                                    <Badge variant="success" className="text-[10px] py-0 px-1.5">
                                      {t("security.sessions.current")}
                                    </Badge>
                                  )}
                                </span>
                                <span className="text-xs text-muted-foreground truncate max-w-xs">{ua}</span>
                              </div>
                            </div>
                          </TableCell>
                          <TableCell>
                            <span className="font-mono text-xs">{ip}</span>
                          </TableCell>
                          <TableCell>
                            <span className="text-xs text-muted-foreground">
                              {formatDate(new Date(activeTime))}
                            </span>
                          </TableCell>
                          <TableCell className="text-right">
                            {!session.current && (
                              <Button
                                variant="ghost"
                                size="sm"
                                onClick={() => handleRevokeSession(session.id)}
                                className="text-destructive hover:text-destructive hover:bg-destructive/10 text-xs h-8 px-2"
                              >
                                {t("security.sessions.revoke")}
                              </Button>
                            )}
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      )}

      {/* TAB 4: LOGIN HISTORY */}
      {activeTab === "history" && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <History className="h-5 w-5 text-primary" />
              {t("security.history.title")}
            </CardTitle>
            <CardDescription>{t("security.history.desc")}</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="rounded-md border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("security.history.date")}</TableHead>
                    <TableHead>{t("security.history.ip")}</TableHead>
                    <TableHead>{t("security.history.client")}</TableHead>
                    <TableHead className="text-right">{t("security.history.status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {historyLoading ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-center py-6 text-muted-foreground text-sm">
                        {t("common.loading")}
                      </TableCell>
                    </TableRow>
                  ) : loginHistory.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-center py-6 text-muted-foreground text-sm">
                        No login events recorded
                      </TableCell>
                    </TableRow>
                  ) : (
                    loginHistory.map((evt) => (
                      <TableRow key={evt.id}>
                        <TableCell className="text-xs text-muted-foreground">
                          {formatDate(new Date(evt.created_at))}
                        </TableCell>
                        <TableCell className="font-mono text-xs">{evt.ip_address}</TableCell>
                        <TableCell className="text-xs">{evt.user_agent}</TableCell>
                        <TableCell className="text-right">
                          {evt.status === "success" && (
                            <Badge variant="success" className="text-[11px]">
                              {t("security.history.success")}
                            </Badge>
                          )}
                          {evt.status === "failed" && (
                            <Badge variant="destructive" className="text-[11px]">
                              {t("security.history.failed")}
                            </Badge>
                          )}
                          {evt.status === "2fa_prompt" && (
                            <Badge variant="outline" className="text-[11px]">
                              {t("security.history.prompt")}
                            </Badge>
                          )}
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
