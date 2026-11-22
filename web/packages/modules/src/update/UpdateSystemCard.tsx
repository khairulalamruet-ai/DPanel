import React, { useCallback, useEffect, useRef, useState } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { RefreshCw, ShieldCheck, ArrowUpCircle, AlertTriangle, CheckCircle2, FileText } from "lucide-react";
import { PanelUpdateStatus, UpdateRun } from "./types";

interface UpdateSystemCardProps {
  status: PanelUpdateStatus;
  onRefresh: () => void | Promise<void>;
}

const POLL_MS = 2000;
// An update takes a few minutes; after this the page stops waiting (the service goes on).
const GIVE_UP_MS = 20 * 60 * 1000;

export function UpdateSystemCard({ status, onRefresh }: UpdateSystemCardProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [isChecking, setIsChecking] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [password, setPassword] = useState("");
  const [isStarting, setIsStarting] = useState(false);
  const [autoPassword, setAutoPassword] = useState<string | null>(null);
  const [isSavingAuto, setIsSavingAuto] = useState(false);
  // The run this page started: the version and when (a status older than the click is a previous run).
  const [watch, setWatch] = useState<{ version: string; since: number } | null>(null);
  const [run, setRun] = useState<UpdateRun | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopWatching = useCallback(() => {
    if (timer.current) clearInterval(timer.current);
    timer.current = null;
  }, []);

  // Follow the update service until it says it is done (the panel restarts on the way: a failed read is not an answer).
  useEffect(() => {
    if (!watch) return;
    const tick = async () => {
      if (Date.now() - watch.since > GIVE_UP_MS) {
        stopWatching();
        setWatch(null);
        return;
      }
      try {
        const s = await apiFetch<PanelUpdateStatus>("/api/v1/admin/update/status");
        const r = s.run;
        const fresh = r && r.version === watch.version && Date.parse(r.updated_at) >= watch.since - 5000;
        if (fresh && r) {
          setRun(r);
          if (r.state !== "running") {
            stopWatching();
            setWatch(null);
            toast({
              title:
                r.state === "completed"
                  ? t("update.updateSuccess", "Update applied")
                  : r.state === "rolled_back"
                    ? t("update.rollbackNotice", "The old version was put back")
                    : t("update.updateFailed", "The update failed"),
              description: r.message,
              variant: r.state === "completed" ? "default" : "destructive",
            });
            await onRefresh();
          }
        }
      } catch {
        /* the panel is restarting */
      }
    };
    timer.current = setInterval(() => void tick(), POLL_MS);
    return stopWatching;
  }, [watch, onRefresh, stopWatching, t, toast]);

  const handleCheckUpdates = async () => {
    setIsChecking(true);
    try {
      await apiFetch("/api/v1/admin/update/check", { method: "POST" });
      toast({
        title: t("update.checkSuccess", "Update check complete"),
        description: t("update.checkSuccessDesc", "The release site was asked for its newest release."),
      });
      await onRefresh();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.details?.detail || err?.message || t("update.checkFailed", "The release site could not be reached."),
        variant: "destructive",
      });
    } finally {
      setIsChecking(false);
    }
  };

  const handleApplyUpdate = async () => {
    const offer = status.available_update;
    if (!offer || !password) return;
    setIsStarting(true);
    try {
      // Changing the panel asks for a recent password check ("sudo mode").
      await apiFetch("/api/v1/auth/sudo", { method: "POST", body: JSON.stringify({ password }) });
      const since = Date.now();
      await apiFetch("/api/v1/admin/update/apply", { method: "POST", body: JSON.stringify({ version: offer.version }) });
      setPassword("");
      setConfirming(false);
      setRun(null);
      setWatch({ version: offer.version, since });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.details?.detail || err?.message || t("update.applyFailed", "The update could not be started."),
        variant: "destructive",
      });
    } finally {
      setIsStarting(false);
    }
  };

  // Switch the automatic update on or off: a setting of the system module, which asks for a recent password check.
  const handleToggleAuto = async () => {
    if (!autoPassword) return;
    setIsSavingAuto(true);
    try {
      await apiFetch("/api/v1/auth/sudo", { method: "POST", body: JSON.stringify({ password: autoPassword }) });
      await apiFetch("/api/v1/admin/settings/system", {
        method: "PUT",
        body: JSON.stringify({ values: { auto_update: !status.auto.enabled } }),
      });
      setAutoPassword(null);
      toast({
        title: status.auto.enabled
          ? t("update.auto.switchedOff", "Automatic updates are off")
          : t("update.auto.switchedOn", "Automatic updates are on"),
      });
      await onRefresh();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.details?.detail || err?.message || t("update.auto.saveFailed", "The setting could not be saved."),
        variant: "destructive",
      });
    } finally {
      setIsSavingAuto(false);
    }
  };

  const isUpdating = watch !== null || run?.state === "running";
  const stepText = (r: UpdateRun | null): string => {
    if (!r) return t("update.steps.waiting", "Waiting for the update service to start...");
    if (r.state === "running") {
      return r.step === "downloading"
        ? t("update.steps.downloading", "Downloading the release...")
        : t("update.steps.verifying", "Checking the release signature and installing...");
    }
    return r.message;
  };

  return (
    <Card className="border bg-card shadow-sm" data-testid="panel-update-card">
      <CardHeader>
        <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <ArrowUpCircle className="h-5 w-5 text-primary" />
              <CardTitle className="text-lg font-bold" data-testid="panel-update-title">
                {t("update.title", "Built Panel updates")}
              </CardTitle>
              <Badge variant="outline" className="font-mono text-xs">
                {status.current_version}
              </Badge>
            </div>
            <CardDescription className="text-xs">
              {t(
                "update.description",
                "A new release is downloaded, checked against its signed manifest and installed; if it does not start, the old version is put back."
              )}
            </CardDescription>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={handleCheckUpdates}
            disabled={isChecking || isUpdating}
            className="gap-2 text-xs h-9"
            data-testid="btn-check-updates"
          >
            <RefreshCw className={`h-4 w-4 ${isChecking ? "animate-spin" : ""}`} />
            <span>{t("update.checkNow", "Check for Updates")}</span>
          </Button>
        </div>
      </CardHeader>

      <CardContent className="space-y-6">
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 p-4 rounded-lg border bg-muted/20 text-xs">
          <div className="space-y-1">
            <span className="text-muted-foreground">{t("update.installedVersion", "Installed Version")}</span>
            <div className="font-mono text-base font-bold text-foreground" data-testid="current-version-display">
              {status.current_version}
            </div>
            <div className="text-[11px] text-muted-foreground">
              {t("update.lastChecked", "Last checked")}:{" "}
              {status.last_checked_at ? new Date(status.last_checked_at).toLocaleString() : t("update.neverChecked", "not yet")}
            </div>
          </div>
          <div className="space-y-1">
            <span className="text-muted-foreground">{t("update.service", "Update service")}</span>
            <div className="pt-1 text-foreground" data-testid="update-service-state">
              {status.can_apply
                ? t("update.serviceReady", "Ready: Update now installs a release from here.")
                : t(
                    "update.serviceMissing",
                    "Not installed on this server. Run the installer once more (it keeps your data) to get one-click updates."
                  )}
            </div>
          </div>
        </div>


        <div className="p-4 rounded-lg border bg-muted/10 space-y-3 text-xs" data-testid="auto-update-card">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <div className="space-y-1">
              <div className="font-semibold text-foreground" data-testid="auto-update-state">
                {status.auto.enabled
                  ? t("update.auto.on", "Automatic updates are on")
                  : t("update.auto.off", "Automatic updates are off")}
              </div>
              <p className="text-muted-foreground">
                {status.auto.enabled
                  ? t("update.auto.onDesc", {
                      days: status.auto.delay_days,
                      hour: String(status.auto.hour_utc).padStart(2, "0"),
                      defaultValue:
                        "A new release is installed by itself {{days}} day(s) after it appears, at about {{hour}}:00 UTC. You are told when it appears.",
                    })
                  : t("update.auto.offDesc", "New releases are only announced; you install them with Update Now.")}
              </p>
              {status.auto.enabled && status.auto.scheduled_for && status.available_update && (
                <p className="text-foreground" data-testid="auto-update-scheduled">
                  {t("update.auto.scheduled", "Version {{version}} will be installed on {{when}}.", {
                    version: status.available_update.version,
                    when: new Date(status.auto.scheduled_for).toLocaleString(),
                  })}
                </p>
              )}
            </div>
            {autoPassword === null && (
              <Button variant="outline" size="sm" className="h-9 text-xs" onClick={() => setAutoPassword("")} data-testid="btn-toggle-auto">
                {status.auto.enabled ? t("update.auto.switchOff", "Switch off") : t("update.auto.switchOn", "Switch on")}
              </Button>
            )}
          </div>
          {autoPassword !== null && (
            <form
              className="flex flex-col sm:flex-row sm:items-end gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                void handleToggleAuto();
              }}
            >
              <div className="flex-1 space-y-1">
                <label htmlFor="auto-update-password" className="font-medium">
                  {t("update.confirmPassword", "Your password, to confirm")}
                </label>
                <Input
                  id="auto-update-password"
                  type="password"
                  autoComplete="current-password"
                  value={autoPassword}
                  onChange={(e) => setAutoPassword(e.target.value)}
                  className="h-9 text-xs"
                />
              </div>
              <div className="flex gap-2">
                <Button type="submit" size="sm" className="h-9 text-xs" disabled={!autoPassword || isSavingAuto} data-testid="btn-confirm-auto">
                  {t("common.save", "Save")}
                </Button>
                <Button type="button" variant="ghost" size="sm" className="h-9 text-xs" onClick={() => setAutoPassword(null)}>
                  {t("common.cancel", "Cancel")}
                </Button>
              </div>
            </form>
          )}
        </div>

        {(watch || run) && (
          <div
            className={`p-4 rounded-lg border text-xs ${
              run?.state === "completed"
                ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-700 dark:text-emerald-300"
                : run?.state === "failed" || run?.state === "rolled_back"
                  ? "bg-destructive/10 border-destructive/30 text-destructive"
                  : "bg-primary/10 border-primary/30 text-foreground"
            }`}
            data-testid="update-progress-container"
          >
            <div className="flex items-center gap-2 font-semibold">
              {isUpdating && <RefreshCw className="h-4 w-4 animate-spin text-primary" />}
              {run?.state === "completed" && <CheckCircle2 className="h-4 w-4 text-emerald-500" />}
              {(run?.state === "failed" || run?.state === "rolled_back") && <AlertTriangle className="h-4 w-4 text-destructive" />}
              <span data-testid="update-step-message">{stepText(run)}</span>
            </div>
            {run?.state === "completed" && (
              <Button size="sm" className="mt-3 h-8 text-xs" onClick={() => window.location.reload()}>
                {t("update.reload", "Reload the page")}
              </Button>
            )}
          </div>
        )}

        {status.available_update ? (
          <div className="p-4 rounded-lg border border-primary/30 bg-primary/5 space-y-4" data-testid="update-available-banner">
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <h4 className="text-sm font-bold text-foreground">
                    {t("update.newVersionAvailable", "New Version Available")}: v{status.available_update.version}
                  </h4>
                  <Badge variant="secondary" className="gap-1 text-[10px]">
                    <ShieldCheck className="h-3 w-3" />
                    <span>{t("update.signedBy", "Signed by")} {status.available_update.key_id}</span>
                  </Badge>
                </div>
                <p className="text-xs text-muted-foreground">
                  {t("update.releasedOn", "Released on")} {new Date(status.available_update.release_date).toLocaleDateString()}
                  {" · "}
                  {t("update.checkedAgain", "The signature is checked again before anything is installed.")}
                </p>
              </div>
              {!confirming && (
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => setConfirming(true)}
                  disabled={isUpdating || !status.can_apply}
                  className="gap-2 text-xs h-9 font-semibold"
                  data-testid="btn-update-now"
                >
                  <ArrowUpCircle className="h-4 w-4" />
                  <span>{t("update.updateNow", "Update Now")}</span>
                </Button>
              )}
            </div>

            {confirming && (
              <form
                className="flex flex-col sm:flex-row sm:items-end gap-3 p-3 rounded border bg-card"
                onSubmit={(e) => {
                  e.preventDefault();
                  void handleApplyUpdate();
                }}
                data-testid="update-confirm"
              >
                <div className="flex-1 space-y-1">
                  <label htmlFor="update-password" className="text-xs font-medium">
                    {t("update.confirmPassword", "Your password, to confirm")}
                  </label>
                  <Input
                    id="update-password"
                    type="password"
                    autoComplete="current-password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    className="h-9 text-xs"
                  />
                  <p className="text-[11px] text-muted-foreground">
                    {t("update.restartNotice", "The panel restarts for a moment; sign-ins in progress stay open.")}
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button type="submit" size="sm" disabled={!password || isStarting} className="h-9 text-xs" data-testid="btn-confirm-update">
                    {t("update.confirmUpdate", "Update to")} v{status.available_update.version}
                  </Button>
                  <Button type="button" variant="ghost" size="sm" className="h-9 text-xs" onClick={() => { setConfirming(false); setPassword(""); }}>
                    {t("common.cancel", "Cancel")}
                  </Button>
                </div>
              </form>
            )}

            {status.available_update.changelog.length > 0 && (
              <div className="space-y-1.5 p-3 rounded border bg-card text-xs">
                <span className="font-semibold text-foreground flex items-center gap-1.5">
                  <FileText className="h-3.5 w-3.5 text-primary" />
                  {t("update.changelog", "Release Notes & Changelog")}
                </span>
                <ul className="list-disc list-inside space-y-1 text-muted-foreground">
                  {status.available_update.changelog.map((item, idx) => (
                    <li key={idx}>{item}</li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        ) : (
          <div className="p-4 rounded-lg border bg-muted/10 flex items-center gap-3 text-xs text-muted-foreground">
            <CheckCircle2 className="h-5 w-5 text-emerald-500 shrink-0" />
            <div>
              <span className="font-semibold text-foreground">{t("update.upToDate", "Built Panel is up to date.")}</span>
              <p>{t("update.upToDateDesc", "No newer release was found at the last check.")}</p>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
