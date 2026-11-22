import React, { useState } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Badge,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  ExternalLink,
  Settings,
  Trash2,
  Search,
  Globe,
  Layers,
  Code,
  FileCode,
  Sparkles,
  ArrowUpCircle,
  KeyRound,
  AlertTriangle,
} from "lucide-react";
import { InstalledApp } from "./types";

interface InstalledAppsTableProps {
  apps: InstalledApp[];
  onOpenManage: (app: InstalledApp) => void;
  /** An application cannot be removed from the panel yet; the button shows when this is given. */
  onUninstall?: (appId: string) => Promise<void>;
}

export function InstalledAppsTable({
  apps,
  onOpenManage,
  onUninstall,
}: InstalledAppsTableProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [appToUninstall, setAppToUninstall] = useState<InstalledApp | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const filtered = apps.filter((app) => {
    if (search.trim()) {
      const q = search.toLowerCase();
      return (
        app.name.toLowerCase().includes(q) ||
        app.domain.toLowerCase().includes(q) ||
        app.url.toLowerCase().includes(q)
      );
    }
    return true;
  });

  const getAppIcon = (catalogId: string) => {
    switch (catalogId) {
      case "wordpress":
        return <Globe className="h-4 w-4 text-blue-600" />;
      case "laravel":
        return <Layers className="h-4 w-4 text-red-600" />;
      case "joomla":
        return <Code className="h-4 w-4 text-amber-600" />;
      case "drupal":
        return <FileCode className="h-4 w-4 text-sky-600" />;
      default:
        return <Sparkles className="h-4 w-4 text-emerald-600" />;
    }
  };

  const getStatusBadge = (status: string) => {
    switch (status) {
      case "running":
        return (
          <Badge variant="outline" className="bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300 border-emerald-300 text-xs">
            {t("apps.statusRunning", "Running")}
          </Badge>
        );
      case "maintenance":
        return (
          <Badge variant="outline" className="bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300 border-amber-300 text-xs">
            {t("apps.statusMaintenance", "Maintenance")}
          </Badge>
        );
      case "error":
      default:
        return (
          <Badge variant="destructive" className="text-xs">
            {t("apps.statusError", "Error")}
          </Badge>
        );
    }
  };

  const confirmUninstall = async () => {
    if (!appToUninstall) return;
    setIsDeleting(true);
    try {
      await onUninstall?.(appToUninstall.id);
      setAppToUninstall(null);
    } catch {
      // handled
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="installed-apps-container">
      {/* Search Header */}
      <div className="flex items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder={t("apps.searchInstalled", "Filter by domain or app name...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8 text-xs"
            data-testid="input-search-installed"
          />
        </div>
      </div>

      {filtered.length === 0 ? (
        <div className="p-8 text-center border rounded-lg bg-card" data-testid="empty-installed-placeholder">
          <p className="text-sm font-semibold">{t("apps.noInstalledApps", "No applications installed yet.")}</p>
          <p className="text-xs text-muted-foreground mt-1">
            {t("apps.noInstalledAppsSubtitle", "Browse the App Store tab above to install WordPress, Joomla, or other frameworks with one click.")}
          </p>
        </div>
      ) : (
        <div className="rounded-lg border bg-card overflow-hidden">
          <Table data-testid="installed-apps-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("apps.appColumn", "Application")}</TableHead>
                <TableHead>{t("apps.locationColumn", "Domain / URL")}</TableHead>
                <TableHead>{t("apps.versionColumn", "Version")}</TableHead>
                <TableHead>{t("apps.statusColumn", "Status")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((app) => (
                <TableRow key={app.id} data-testid={`app-row-${app.id}`}>
                  <TableCell>
                    <div className="flex items-center gap-2.5">
                      <div className="p-2 bg-muted rounded-lg border flex items-center justify-center">
                        {getAppIcon(app.catalog_app_id)}
                      </div>
                      <div>
                        <span className="font-bold text-sm block" data-testid={`installed-app-name-${app.id}`}>
                          {app.name}
                        </span>
                        <span className="text-[11px] text-muted-foreground font-mono">
                          {app.catalog_app_id}
                        </span>
                      </div>
                    </div>
                  </TableCell>

                  <TableCell>
                    <div className="space-y-0.5">
                      <a
                        href={app.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        data-testid={`link-visit-app-${app.id}`}
                        className="text-xs font-mono font-semibold text-primary hover:underline flex items-center gap-1"
                      >
                        {app.url}
                        <ExternalLink className="h-3 w-3" />
                      </a>
                      <span className="text-[10px] text-muted-foreground font-mono block">
                        {app.domain} ({app.path})
                      </span>
                    </div>
                  </TableCell>

                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <span className="font-mono text-xs">v{app.version}</span>
                      {app.has_update && (
                        <Badge variant="outline" className="text-[10px] bg-amber-50 text-amber-700 border-amber-300 flex items-center gap-1">
                          <ArrowUpCircle className="h-3 w-3 text-amber-600" />
                          v{app.latest_version}
                        </Badge>
                      )}
                    </div>
                  </TableCell>

                  <TableCell>{getStatusBadge(app.status)}</TableCell>

                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-1.5">
                      {app.admin_url && (
                        <a
                          href={app.admin_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          data-testid={`btn-admin-login-${app.id}`}
                          className="h-8 px-2.5 inline-flex items-center justify-center rounded-md text-xs font-medium border border-input bg-background hover:bg-accent hover:text-accent-foreground transition-colors"
                        >
                          <KeyRound className="h-3.5 w-3.5 mr-1" />
                          {t("apps.adminLogin", "Admin")}
                        </a>
                      )}

                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => onOpenManage(app)}
                        data-testid={`btn-manage-app-${app.id}`}
                        className="h-8 text-xs font-semibold flex items-center gap-1"
                      >
                        <Settings className="h-3.5 w-3.5" />
                        <span>{t("apps.manage", "Manage")}</span>
                      </Button>

                      {onUninstall && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setAppToUninstall(app)}
                        data-testid={`btn-uninstall-app-${app.id}`}
                        className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Confirmation Dialog for Uninstall */}
      <Dialog open={Boolean(appToUninstall)} onOpenChange={(val) => !val && setAppToUninstall(null)}>
        <DialogContent data-testid="uninstall-confirm-dialog" className="sm:max-w-md">
          <DialogHeader>
            <div className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              <DialogTitle>{t("apps.uninstallTitle", "Uninstall Application")}</DialogTitle>
            </div>
            <DialogDescription className="text-xs">
              {t(
                "apps.uninstallWarning",
                "Are you sure you want to uninstall {{name}} on {{url}}? Files and database tables will be removed.",
                { name: appToUninstall?.name, url: appToUninstall?.url }
              )}
            </DialogDescription>
          </DialogHeader>

          <DialogFooter>
            <Button variant="outline" onClick={() => setAppToUninstall(null)} disabled={isDeleting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={confirmUninstall}
              disabled={isDeleting}
              data-testid="btn-confirm-uninstall"
            >
              {isDeleting ? t("apps.uninstalling", "Uninstalling...") : t("apps.confirmUninstall", "Uninstall Application")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
