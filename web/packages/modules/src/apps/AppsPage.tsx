import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { listCatalog, listInstalled, installApp, forgetApp, loadWordPress, setMaintenance, updatePlugin, scanWordPress, hardenWordPress } from "./api";
import { useToast, Button } from "@builtpanel/ui";
import {
  Boxes,
  Download,
  ListFilter,
  RefreshCw,
  Plus,
  AlertCircle,
} from "lucide-react";
import { CatalogApp, InstalledApp, WordPressDetails, InstallAppPayload } from "./types";
import { CatalogGrid } from "./CatalogGrid";
import { InstallWizardDialog } from "./InstallWizardDialog";
import { InstalledAppsTable } from "./InstalledAppsTable";
import { WordPressManagerDialog } from "./WordPressManagerDialog";

export function AppsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"installed" | "catalog">("installed");
  const [catalog, setCatalog] = useState<CatalogApp[]>([]);
  const [installedApps, setInstalledApps] = useState<InstalledApp[]>([]);
  const [domainChoices, setDomainChoices] = useState<{ id: string; name: string }[]>([]);
  const [domains, setDomains] = useState<string[]>(["client-site.com", "myblog.net"]);
  const [isLoading, setIsLoading] = useState(true);
  const [isError, setIsError] = useState(false);

  // Modal states
  const [selectedCatalogApp, setSelectedCatalogApp] = useState<CatalogApp | null>(null);
  const [managingApp, setManagingApp] = useState<InstalledApp | null>(null);
  const [wpDetails, setWpDetails] = useState<WordPressDetails | null>(null);

  const fetchAppsData = useCallback(async () => {
    try {
      setIsLoading(true);
      setIsError(false);

      const [catalogRes, installedRes, domainRes] = await Promise.all([
        listCatalog(),
        listInstalled(),
        apiFetch<{ items: { id: string; fqdn: string; unicode: string }[] }>("/api/v1/client/domains").catch(() => ({ items: [] })),
      ]);
      setDomainChoices(domainRes.items.map((d) => ({ id: d.id, name: d.unicode || d.fqdn })));
      setCatalog(catalogRes);
      setInstalledApps(installedRes);

      setDomains(domainRes.items.map((d) => d.unicode || d.fqdn));
    } catch {
      setIsError(true);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchAppsData();
  }, [fetchAppsData]);

  // Handle Install Application
  const handleInstallApp = async (payload: InstallAppPayload): Promise<InstalledApp> => {
    const domain = domainChoices.find((d) => d.name === payload.domain);
    if (!domain) throw new Error("Pick one of your domains");
    const app = await installApp({ ...payload, domain_id: domain.id });
    const res = { app };

    setInstalledApps((prev) => [res.app, ...prev]);

    toast({
      title: t("apps.installSuccessToastTitle", "Application Installed"),
      description: t("apps.installSuccessToastDesc", "{{name}} is now active on {{domain}}", {
        name: res.app.name,
        domain: res.app.domain,
      }),
    });

    return res.app;
  };

  // Open Manage dialog
  const handleOpenManage = async (app: InstalledApp) => {
    setManagingApp(app);
    if (app.catalog_app_id === "wordpress") {
      try {
        setWpDetails(await loadWordPress(app.id));
      } catch (err: any) {
        setWpDetails(null);
        toast({
          title: t("common.error"),
          description: err?.message || t("common.unknownError"),
          variant: "destructive",
        });
      }
    }
  };

  // Update WordPress Plugin (T12.03)
  const handleUpdatePlugin = async (appId: string, slug: string) => {
    setWpDetails(await updatePlugin(appId, slug, wpDetails));

    toast({
      title: t("wpManager.pluginUpdatedTitle", "Plugin Updated"),
      description: t("wpManager.pluginUpdatedDesc", "Plugin {{slug}} has been updated to the latest version.", { slug }),
    });
  };

  // Toggle Maintenance Mode
  const handleToggleMaintenance = async (appId: string, enabled: boolean) => {
    setWpDetails(await setMaintenance(appId, enabled, wpDetails));
    setInstalledApps((prev) =>
      prev.map((a) => (a.id === appId ? { ...a, status: enabled ? "maintenance" : "running" } : a))
    );

    toast({
      title: enabled ? t("wpManager.maintenanceEnabled", "Maintenance Mode Active") : t("wpManager.maintenanceDisabled", "Maintenance Mode Disabled"),
    });
  };

  const handleScan = async (appId: string) => {
    setWpDetails(await scanWordPress(appId, wpDetails));
  };

  const handleHarden = async (appId: string) => {
    setWpDetails(await hardenWordPress(appId, wpDetails));
    toast({ title: t("wpManager.hardened", "The safety settings were put right") });
  };

  if (isLoading) {
    return (
      <div className="flex h-64 items-center justify-center" data-testid="apps-page-loading">
        <p className="text-sm text-muted-foreground">{t("common.loading", "Loading...")}</p>
      </div>
    );
  }

  if (isError) {
    return (
      <div className="p-8 text-center" data-testid="apps-page-error">
        <AlertCircle className="h-10 w-10 text-destructive mx-auto mb-2" />
        <p className="font-semibold">{t("apps.loadError", "Failed to load applications.")}</p>
        <button
          onClick={() => fetchAppsData()}
          className="mt-4 px-4 py-2 bg-primary text-primary-foreground rounded text-xs font-semibold"
        >
          {t("common.retry", "Retry")}
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-6" data-testid="apps-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2" data-testid="apps-page-title">
            <Boxes className="h-7 w-7 text-primary" />
            {t("apps.pageTitle", "Applications & Autoinstaller")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("apps.pageSubtitle", "Install, manage, and automate WordPress, Joomla, Laravel, and other web apps.")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            onClick={() => setActiveTab("catalog")}
            data-testid="btn-open-catalog-tab"
            className="flex items-center gap-1.5 h-9 text-xs font-semibold"
          >
            <Plus className="h-4 w-4" />
            <span>{t("apps.installNewApp", "Install Application")}</span>
          </Button>
        </div>
      </div>

      {/* Main Tabs (Installed vs Catalog) - T12.06 DoD */}
      <div className="flex items-center border-b space-x-4">
        <button
          onClick={() => setActiveTab("installed")}
          data-testid="tab-installed-apps"
          className={`pb-3 font-semibold text-sm transition-all border-b-2 flex items-center gap-2 ${
            activeTab === "installed"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <ListFilter className="h-4 w-4" />
          <span>{t("apps.tabInstalled", "Installed Applications")}</span>
          <span className="bg-muted px-2 py-0.5 rounded-full text-xs font-mono font-medium">
            {installedApps.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("catalog")}
          data-testid="tab-app-catalog"
          className={`pb-3 font-semibold text-sm transition-all border-b-2 flex items-center gap-2 ${
            activeTab === "catalog"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Download className="h-4 w-4" />
          <span>{t("apps.tabCatalog", "App Store / Catalog")}</span>
          <span className="bg-muted px-2 py-0.5 rounded-full text-xs font-mono font-medium">
            {catalog.length}
          </span>
        </button>
      </div>

      {/* Tab Panels */}
      {activeTab === "installed" ? (
        <InstalledAppsTable
          apps={installedApps}
          onOpenManage={handleOpenManage}
          onUninstall={async (id) => {
            try {
              await forgetApp(id);
              setInstalledApps((prev) => prev.filter((a) => a.id !== id));
              toast({ title: t("apps.removed", "Application removed from the list; its files stay in your account") });
            } catch (e) {
              toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
            }
          }}
        />
      ) : (
        <CatalogGrid
          catalog={catalog}
          onSelectApp={(app) => setSelectedCatalogApp(app)}
        />
      )}

      {/* Install Wizard Dialog (T12.06) */}
      <InstallWizardDialog
        open={Boolean(selectedCatalogApp)}
        app={selectedCatalogApp}
        domains={domains}
        onClose={() => setSelectedCatalogApp(null)}
        onInstall={handleInstallApp}
        onSuccess={() => {
          setActiveTab("installed");
        }}
      />

      {/* WordPress Manager Dialog (T12.03) */}
      <WordPressManagerDialog
        open={Boolean(managingApp)}
        app={managingApp}
        details={wpDetails}
        onClose={() => {
          setManagingApp(null);
          setWpDetails(null);
        }}
        onUpdatePlugin={handleUpdatePlugin}
        onToggleMaintenance={handleToggleMaintenance}
        onScan={handleScan}
        onHarden={handleHarden}
      />
    </div>
  );
}
