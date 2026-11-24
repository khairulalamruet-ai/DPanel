import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, CardContent, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { Download, Trash2, ExternalLink } from "lucide-react";
import { AdminSoftware } from "./types";
import { SoftwareIcon } from "./SoftwareIcon";

const BASE = "/api/v1/admin/marketplace";

export function AppInstallerPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [apps, setApps] = useState<AdminSoftware[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [category, setCategory] = useState("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setApps(await apiFetch<AdminSoftware[]>(BASE));
      setError(null);
    } catch (err: any) {
      setError(err?.message || t("appInstaller.loadError", "The app list could not be loaded."));
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const categories = useMemo(() => {
    const seen = new Map<string, string>();
    for (const a of apps ?? []) if (!seen.has(a.category)) seen.set(a.category, a.category_name);
    return [...seen.entries()];
  }, [apps]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (apps ?? []).filter(
      (a) =>
        (category === "all" || a.category === category) &&
        (!q || a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q)),
    );
  }, [apps, category, query]);

  const change = async (app: AdminSoftware, install: boolean) => {
    setBusy(app.slug);
    try {
      await apiFetch(`${BASE}/${encodeURIComponent(app.slug)}`, { method: install ? "POST" : "DELETE" });
      toast({
        title: install
          ? t("appInstaller.installed", "{{name}} is now offered to customers.", { name: app.name })
          : t("appInstaller.removed", "{{name}} is no longer offered.", { name: app.name }),
      });
      await load();
    } catch (err: any) {
      toast({ title: err?.message || t("appInstaller.failed", "The change failed."), variant: "destructive" });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-6" data-testid="app-installer-page">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t("appInstaller.title", "Install an App")}</h1>
        <p className="text-sm text-muted-foreground">
          {t(
            "appInstaller.subtitle",
            "Choose which software customers can install in their own isolated container. Removing one stops new installs; what customers already run keeps running.",
          )}
        </p>
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("appInstaller.search", "Search apps")}
          aria-label={t("appInstaller.search", "Search apps")}
          className="sm:max-w-xs"
          data-testid="app-search"
        />
        <div className="flex flex-wrap gap-2" role="tablist" aria-label={t("appInstaller.categories", "Categories")}>
          {[["all", t("appInstaller.all", "All")] as [string, string], ...categories].map(([key, label]) => (
            <Button
              key={key}
              size="sm"
              variant={category === key ? "default" : "outline"}
              role="tab"
              aria-selected={category === key}
              onClick={() => setCategory(key)}
              data-testid={`category-${key}`}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>

      {!apps ? (
        <p className="text-sm text-muted-foreground" data-testid="app-installer-loading">
          {error ?? t("common.loading", "Loading...")}
        </p>
      ) : shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("appInstaller.none", "No app matches.")}</p>
      ) : (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {shown.map((app) => (
            <Card key={app.slug} data-testid={`app-${app.slug}`}>
              <CardContent className="flex gap-3 p-4">
                <SoftwareIcon name={app.name} />
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{app.name}</span>
                    {app.version && <span className="text-xs text-muted-foreground">{app.version}</span>}
                    {app.offered && <Badge data-testid={`offered-${app.slug}`}>{t("appInstaller.offered", "Offered")}</Badge>}
                  </div>
                  <p className="line-clamp-2 text-sm text-muted-foreground">{app.description}</p>
                  <div className="flex flex-wrap items-center gap-2 pt-1 text-xs text-muted-foreground">
                    <span>{app.category_name}</span>
                    <span>·</span>
                    <span>{app.license}</span>
                    {app.kind === "container" && (
                      <>
                        <span>·</span>
                        <span>{app.memory_mb} MiB</span>
                      </>
                    )}
                    {app.homepage && (
                      <a href={app.homepage} target="_blank" rel="noreferrer noopener" aria-label={t("appInstaller.homepage", "Project page")}>
                        <ExternalLink className="h-3 w-3" />
                      </a>
                    )}
                  </div>
                  <div className="pt-2">
                    {app.kind === "container" ? (
                      app.offered ? (
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={busy === app.slug}
                          onClick={() => change(app, false)}
                          data-testid={`remove-${app.slug}`}
                        >
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                          {t("appInstaller.remove", "Remove")}
                        </Button>
                      ) : (
                        <Button size="sm" disabled={busy === app.slug} onClick={() => change(app, true)} data-testid={`install-${app.slug}`}>
                          <Download className="mr-1 h-3.5 w-3.5" />
                          {t("appInstaller.install", "Install")}
                        </Button>
                      )
                    ) : (
                      <span className="text-xs text-muted-foreground">
                        {app.kind === "builtin"
                          ? t("appInstaller.builtin", "Part of the panel: {{where}}", { where: app.provided_by })
                          : t("appInstaller.unavailable", "Not available: {{why}}", { why: app.reason })}
                      </span>
                    )}
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
