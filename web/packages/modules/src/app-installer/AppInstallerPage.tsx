import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, CardContent, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { Download, Trash2, ExternalLink, Search, Sparkles, Check, HardDrive } from "lucide-react";
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
      {/* Page Header */}
      <div className="flex flex-col gap-1 border-b pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight">{t("appInstaller.title", "Install an App")}</h1>
          <Badge variant="outline" className="text-xs bg-primary/10 text-primary border-primary/20">
            Marketplace
          </Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          {t(
            "appInstaller.subtitle",
            "Choose which software customers can install in their own isolated container. Removing one stops new installs; what customers already run keeps running.",
          )}
        </p>
      </div>

      {/* Featured Quick-Picks Strip */}
      {apps && apps.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            <Sparkles className="h-3.5 w-3.5 text-amber-500" />
            <span>Featured Software</span>
          </div>
          <div className="flex items-center gap-2.5 overflow-x-auto pb-2 scrollbar-thin">
            {apps.slice(0, 8).map((a) => {
              const isSelected = category === a.category && !query;
              return (
                <button
                  key={a.slug}
                  type="button"
                  onClick={() => {
                    setCategory(a.category);
                    setQuery("");
                  }}
                  className={`flex items-center gap-2.5 px-3 py-2 rounded-xl border bg-card hover:bg-accent/40 transition-all shrink-0 cursor-pointer text-left ${
                    isSelected ? "border-primary ring-1 ring-primary/30 bg-primary/5" : "border-border"
                  }`}
                >
                  <SoftwareIcon slug={a.slug} name={a.name} size={36} />
                  <div className="min-w-0">
                    <p className="text-xs font-bold truncate max-w-[100px] leading-tight">{a.name}</p>
                    <p className="text-[10px] text-muted-foreground truncate">{a.category_name}</p>
                  </div>
                  {a.offered && (
                    <span className="ml-1 inline-flex items-center justify-center h-4 w-4 rounded-full bg-emerald-500/15 text-emerald-600">
                      <Check className="h-2.5 w-2.5" />
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Search and Category Filters */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center justify-between">
        <div className="relative w-full sm:max-w-xs">
          <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("appInstaller.search", "Search apps")}
            aria-label={t("appInstaller.search", "Search apps")}
            className="pl-9 text-xs h-9"
            data-testid="app-search"
          />
        </div>
        <div className="flex flex-wrap gap-1.5 overflow-x-auto pb-1 sm:pb-0" role="tablist" aria-label={t("appInstaller.categories", "Categories")}>
          {[["all", t("appInstaller.all", "All")] as [string, string], ...categories].map(([key, label]) => (
            <Button
              key={key}
              size="sm"
              variant={category === key ? "default" : "outline"}
              role="tab"
              aria-selected={category === key}
              onClick={() => setCategory(key)}
              className={`h-8 text-xs font-medium ${category === key ? "shadow-xs font-bold" : "text-muted-foreground"}`}
              data-testid={`category-${key}`}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>

      {/* Apps Cards Grid */}
      {!apps ? (
        <div className="p-8 text-center border rounded-xl bg-card" data-testid="app-installer-loading">
          <p className="text-sm text-muted-foreground">{error ?? t("common.loading", "Loading...")}</p>
        </div>
      ) : shown.length === 0 ? (
        <div className="p-8 text-center border rounded-xl bg-card">
          <p className="text-sm text-muted-foreground">{t("appInstaller.none", "No app matches.")}</p>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {shown.map((app) => (
            <Card
              key={app.slug}
              data-testid={`app-${app.slug}`}
              className="flex flex-col justify-between hover:shadow-md transition-all duration-200 border overflow-hidden"
            >
              {/* Card Header */}
              <div className="p-4 pb-3 border-b bg-muted/20 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="font-bold text-base truncate">{app.name}</span>
                  {app.version && (
                    <Badge variant="secondary" className="text-[10px] font-mono px-1.5 py-0.2 shrink-0">
                      v{app.version}
                    </Badge>
                  )}
                  {app.offered && (
                    <Badge
                      data-testid={`offered-${app.slug}`}
                      className="bg-emerald-500 hover:bg-emerald-600 text-white text-[10px] px-1.5 py-0.2 font-medium shrink-0"
                    >
                      {t("appInstaller.offered", "Offered")}
                    </Badge>
                  )}
                </div>

                {app.homepage && (
                  <a
                    href={app.homepage}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="text-muted-foreground hover:text-primary transition-colors shrink-0"
                    aria-label={t("appInstaller.homepage", "Project page")}
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                  </a>
                )}
              </div>

              {/* Card Body */}
              <CardContent className="p-4 flex-1 flex flex-col justify-between gap-3">
                <div className="flex items-start gap-3.5">
                  <SoftwareIcon slug={app.slug} name={app.name} size={56} />
                  <div className="min-w-0 flex-1 space-y-1.5">
                    <p className="line-clamp-2 text-xs leading-relaxed text-muted-foreground">{app.description}</p>
                    <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                      <Badge variant="outline" className="text-[10px] font-normal px-1.5 py-0">
                        {app.category_name}
                      </Badge>
                      <span>·</span>
                      <span className="font-mono text-[10px]">{app.license}</span>
                      {app.kind === "container" && app.memory_mb > 0 && (
                        <>
                          <span>·</span>
                          <span className="inline-flex items-center gap-1 font-medium text-[10px]">
                            <HardDrive className="h-3 w-3 text-muted-foreground" />
                            {app.memory_mb} MiB
                          </span>
                        </>
                      )}
                    </div>
                  </div>
                </div>

                {/* Card Action / Footer */}
                <div className="pt-2 border-t mt-1">
                  {app.kind === "container" ? (
                    app.offered ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="w-full h-8 text-xs font-semibold border-red-200 text-red-600 hover:bg-red-50 hover:text-red-700 dark:border-red-900/40 dark:text-red-400 dark:hover:bg-red-950/40"
                        disabled={busy === app.slug}
                        onClick={() => change(app, false)}
                        data-testid={`remove-${app.slug}`}
                      >
                        <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                        {t("appInstaller.remove", "Remove")}
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        className="w-full h-8 text-xs font-semibold"
                        disabled={busy === app.slug}
                        onClick={() => change(app, true)}
                        data-testid={`install-${app.slug}`}
                      >
                        <Download className="mr-1.5 h-3.5 w-3.5" />
                        {t("appInstaller.install", "Install")}
                      </Button>
                    )
                  ) : (
                    <div className="py-1 px-2.5 rounded-lg bg-muted/40 border border-muted text-xs text-muted-foreground">
                      {app.kind === "builtin"
                        ? t("appInstaller.builtin", "Part of the panel: {{where}}", { where: app.provided_by })
                        : t("appInstaller.unavailable", "Not available: {{why}}", { why: app.reason })}
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
