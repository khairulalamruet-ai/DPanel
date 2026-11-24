import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Download,
  CheckCircle,
  ExternalLink,
  Lock,
  Globe,
  Database,
  User,
  Mail,
  Key,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import { CatalogApp, InstallAppPayload, InstalledApp } from "./types";

interface InstallWizardDialogProps {
  open: boolean;
  app: CatalogApp | null;
  domains: string[];
  onClose: () => void;
  onInstall: (payload: InstallAppPayload) => Promise<InstalledApp>;
  onSuccess: (installedApp: InstalledApp) => void;
}

export function InstallWizardDialog({
  open,
  app,
  domains,
  onClose,
  onInstall,
  onSuccess,
}: InstallWizardDialogProps) {
  const { t } = useTranslation();

  const [selectedDomain, setSelectedDomain] = useState(domains[0] || "");
  const [protocol, setProtocol] = useState<"https" | "http">("https");
  const [installPath, setInstallPath] = useState("");
  const [siteName, setSiteName] = useState("My Awesome Site");
  const [adminUsername, setAdminUsername] = useState("admin");
  const [adminEmail, setAdminEmail] = useState("");
  const [adminPassword, setAdminPassword] = useState("");
  const [autoUpdate, setAutoUpdate] = useState(true);

  // Installation state
  const [isInstalling, setIsInstalling] = useState(false);
  const [installResult, setInstallResult] = useState<InstalledApp | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  if (!app) return null;

  const handleGeneratePassword = () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
    let pass = "";
    for (let i = 0; i < 16; i++) {
      pass += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    setAdminPassword(pass);
  };

  const handleInstallSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);
    setIsInstalling(true);

    try {
      const payload: InstallAppPayload = {
        catalog_app_id: app.id,
        domain: selectedDomain,
        path: installPath ? (installPath.startsWith("/") ? installPath : `/${installPath}`) : "/",
        protocol,
        site_name: siteName,
        admin_username: adminUsername,
        admin_email: adminEmail,
        admin_password: adminPassword,
        auto_update: autoUpdate,
      };

      const installed = await onInstall(payload);
      setInstallResult(installed);
      onSuccess(installed);
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to install application");
    } finally {
      setIsInstalling(false);
    }
  };

  const handleResetAndClose = () => {
    setInstallResult(null);
    setErrorMsg(null);
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && handleResetAndClose()}>
      <DialogContent data-testid="install-wizard-dialog" className="sm:max-w-lg">
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="p-2.5 rounded-xl bg-primary/10 text-primary">
              <Download className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="install-wizard-title" className="text-lg">
                {installResult
                  ? t("apps.installSuccessTitle", "Installation Completed!")
                  : t("apps.installTitle", "Install {{name}}", { name: app.name })}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {installResult
                  ? t("apps.installSuccessSubtitle", "Your application is live and ready to use.")
                  : t("apps.wizardSubtitle", "Configure domain URL, administrative credentials, and database settings.")}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {installResult ? (
          <div className="space-y-4 py-3" data-testid="install-success-view">
            <div className="p-4 bg-emerald-50 dark:bg-emerald-950/30 border border-emerald-200 dark:border-emerald-800 rounded-lg text-center space-y-2">
              <CheckCircle className="h-10 w-10 text-emerald-600 dark:text-emerald-400 mx-auto" />
              <h3 className="font-bold text-sm text-emerald-900 dark:text-emerald-200">
                {app.name} {t("apps.hasBeenInstalled", "has been successfully installed")}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t("apps.credentialsSavedNotice", "Keep your administrator credentials safe. Database and configuration files were automatically prepared.")}
              </p>
            </div>

            <div className="p-3 bg-muted/60 border rounded-lg space-y-2 text-xs">
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">{t("apps.siteUrl", "Site URL")}:</span>
                <a
                  href={installResult.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  data-testid="link-installed-site"
                  className="font-mono text-primary flex items-center gap-1 hover:underline font-semibold"
                >
                  {installResult.url}
                  <ExternalLink className="h-3 w-3" />
                </a>
              </div>

              {installResult.admin_url && (
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("apps.adminUrl", "Administrative URL")}:</span>
                  <a
                    href={installResult.admin_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    data-testid="link-installed-admin"
                    className="font-mono text-primary flex items-center gap-1 hover:underline font-semibold"
                  >
                    {installResult.admin_url}
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </div>
              )}

              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">{t("apps.adminUser", "Admin User")}:</span>
                <span className="font-mono font-medium">{adminUsername}</span>
              </div>
            </div>

            <DialogFooter>
              <Button onClick={handleResetAndClose} data-testid="btn-finish-install" className="w-full">
                {t("apps.viewInstalledApps", "View Installed Applications")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={handleInstallSubmit} className="space-y-4 py-2">
            {errorMsg && (
              <div className="p-3 bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300 rounded-lg text-xs">
                {errorMsg}
              </div>
            )}

            {/* Installation URL */}
            <div className="space-y-2 border p-3 rounded-lg bg-card">
              <span className="text-xs font-bold flex items-center gap-1.5">
                <Globe className="h-3.5 w-3.5 text-primary" />
                {t("apps.installLocation", "Installation Location")}
              </span>

              <div className="grid grid-cols-3 gap-2">
                <div className="col-span-1">
                  <label className="text-[11px] text-muted-foreground block mb-1">
                    {t("apps.protocol", "Protocol")}
                  </label>
                  <select
                    value={protocol}
                    onChange={(e) => setProtocol(e.target.value as any)}
                    className="w-full border rounded-md h-8 text-xs px-2 bg-background"
                    data-testid="select-app-protocol"
                  >
                    <option value="https">https://</option>
                    <option value="http">http://</option>
                  </select>
                </div>

                <div className="col-span-2">
                  <label className="text-[11px] text-muted-foreground block mb-1">
                    {t("apps.chooseDomain", "Domain")}
                  </label>
                  <select
                    value={selectedDomain}
                    onChange={(e) => setSelectedDomain(e.target.value)}
                    className="w-full border rounded-md h-8 text-xs px-2 bg-background font-mono"
                    data-testid="select-app-domain"
                  >
                    {domains.map((dom) => (
                      <option key={dom} value={dom}>
                        {dom}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div>
                <label className="text-[11px] text-muted-foreground block mb-1">
                  {t("apps.directory", "Directory / In Directory (leave empty for root)")}
                </label>
                <div className="flex items-center">
                  <span className="bg-muted px-2 py-1 text-xs border border-r-0 rounded-l font-mono text-muted-foreground">
                    /
                  </span>
                  <Input
                    type="text"
                    placeholder="e.g. blog or store"
                    value={installPath}
                    onChange={(e) => setInstallPath(e.target.value.replace(/^\//, ""))}
                    className="rounded-l-none text-xs h-8 font-mono"
                    data-testid="input-install-path"
                  />
                </div>
              </div>
            </div>

            {/* Site & Admin Credentials */}
            <div className="space-y-2 border p-3 rounded-lg bg-card">
              <span className="text-xs font-bold flex items-center gap-1.5">
                <User className="h-3.5 w-3.5 text-primary" />
                {t("apps.siteAndAdminSettings", "Site & Administrative Account")}
              </span>

              <div>
                <label className="text-[11px] text-muted-foreground block mb-1">
                  {t("apps.siteName", "Site Name")}
                </label>
                <Input
                  type="text"
                  value={siteName}
                  onChange={(e) => setSiteName(e.target.value)}
                  className="text-xs h-8"
                  required
                  data-testid="input-site-name"
                />
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-[11px] text-muted-foreground block mb-1">
                    {t("apps.adminUsername", "Admin Username")}
                  </label>
                  <Input
                    type="text"
                    value={adminUsername}
                    onChange={(e) => setAdminUsername(e.target.value)}
                    className="text-xs h-8"
                    required
                    data-testid="input-admin-username"
                  />
                </div>

                <div>
                  <label className="text-[11px] text-muted-foreground block mb-1">
                    {t("apps.adminEmail", "Admin Email")}
                  </label>
                  <Input
                    type="email"
                    value={adminEmail}
                    onChange={(e) => setAdminEmail(e.target.value)}
                    className="text-xs h-8"
                    required
                    data-testid="input-admin-email"
                  />
                </div>
              </div>

              <div>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-[11px] text-muted-foreground">
                    {t("apps.adminPassword", "Admin Password")}
                  </label>
                  <button
                    type="button"
                    onClick={handleGeneratePassword}
                    className="text-[10px] text-primary hover:underline flex items-center gap-0.5"
                    data-testid="btn-generate-password"
                  >
                    <RefreshCw className="h-2.5 w-2.5" />
                    {t("apps.generatePassword", "Generate")}
                  </button>
                </div>
                <Input
                  type="text"
                  value={adminPassword}
                  onChange={(e) => setAdminPassword(e.target.value)}
                  className="text-xs h-8 font-mono"
                  required
                  data-testid="input-admin-password"
                />
              </div>
            </div>

            {/* Database & Automation */}
            {app.requires_db && (
              <div className="p-3 bg-muted/40 rounded-lg border text-xs flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Database className="h-4 w-4 text-muted-foreground" />
                  <div>
                    <span className="font-semibold block">{t("apps.autoDbTitle", "Automated MySQL Database")}</span>
                    <span className="text-[11px] text-muted-foreground">
                      {t("apps.autoDbDesc", "Database, user, and secure credentials will be generated automatically.")}
                    </span>
                  </div>
                </div>
                <Badge variant="secondary" className="font-mono text-[10px]">AUTO</Badge>
              </div>
            )}

            <DialogFooter className="pt-2">
              <Button type="button" variant="outline" onClick={handleResetAndClose} disabled={isInstalling}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                type="submit"
                disabled={isInstalling}
                data-testid="btn-submit-install"
                className="flex items-center gap-1.5"
              >
                {isInstalling ? (
                  <>
                    <RefreshCw className="h-4 w-4 animate-spin" />
                    <span>{t("apps.installing", "Installing...")}</span>
                  </>
                ) : (
                  <>
                    <Download className="h-4 w-4" />
                    <span>{t("apps.installNow", "Install")}</span>
                  </>
                )}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
