import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Badge,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Search,
  Download,
  Database,
  ExternalLink,
  Code,
  FileCode,
  Layers,
  Sparkles,
  Globe,
  Star,
} from "lucide-react";
import { CatalogApp } from "./types";

const APP_LOGOS: Record<string, string> = {
  wordpress: "/apps/wordpress.png",
  wp: "/apps/wordpress.png",
  laravel: "/apps/laravel.png",
  joomla: "/apps/joomla.png",
  drupal: "/apps/drupal.png",
  static: "/apps/static.png",
  prestashop: "/apps/prestashop.png",
  moodle: "/apps/moodle.png",
  roundcube: "/apps/roundcube.png",
  whmcs: "/apps/whmcs.png",
};

const APP_BANNERS: Record<string, string> = {
  wordpress: "/apps/wordpress-banner.jpg",
  roundcube: "/apps/roundcube-banner.jpg",
};

interface CatalogGridProps {
  catalog: CatalogApp[];
  onSelectApp: (app: CatalogApp) => void;
}

export function CatalogGrid({ catalog, onSelectApp }: CatalogGridProps) {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string>("all");

  const isBn = i18n.language?.startsWith("bn");

  const categories: { id: string; labelKey: string; defaultLabel: string }[] = [
    { id: "all", labelKey: "common.all", defaultLabel: "All Categories" },
    { id: "cms", labelKey: "apps.catCms", defaultLabel: "CMS & Blogs" },
    { id: "framework", labelKey: "apps.catFramework", defaultLabel: "Frameworks" },
    { id: "static", labelKey: "apps.catStatic", defaultLabel: "Static Sites" },
  ];

  const filteredApps = catalog.filter((app) => {
    if (selectedCategory !== "all" && app.category !== selectedCategory) return false;
    if (search.trim()) {
      const q = search.toLowerCase();
      const matchName = app.name.toLowerCase().includes(q);
      const matchDesc = app.description.toLowerCase().includes(q) || (app.description_bn && app.description_bn.toLowerCase().includes(q));
      return matchName || matchDesc;
    }
    return true;
  });

  const getFallbackIcon = (iconName: string) => {
    switch (iconName) {
      case "wordpress":
        return <Globe className="h-6 w-6 text-blue-600" />;
      case "laravel":
        return <Layers className="h-6 w-6 text-red-600" />;
      case "joomla":
        return <Code className="h-6 w-6 text-amber-600" />;
      case "drupal":
        return <FileCode className="h-6 w-6 text-sky-600" />;
      case "static":
      default:
        return <Sparkles className="h-6 w-6 text-emerald-600" />;
    }
  };

  return (
    <div className="space-y-6" data-testid="catalog-grid-container">
      {/* Featured App Showcase (WordPress Hero Banner) */}
      {!search && selectedCategory === "all" && (
        <div className="relative overflow-hidden rounded-2xl border bg-gradient-to-r from-blue-600/10 via-indigo-600/5 to-transparent p-5 sm:p-6 shadow-xs">
          <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-6">
            <div className="flex items-start gap-4">
              <div className="app-logo-box shrink-0 flex items-center justify-center rounded-2xl bg-white dark:bg-card p-3 shadow-md border border-slate-200 dark:border-slate-800 h-20 w-20">
                <img src="/apps/wordpress.png" alt="WordPress" className="max-h-full max-w-full object-contain" />
              </div>
              <div className="space-y-1.5">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge className="bg-blue-600 text-white font-medium text-[11px] px-2 py-0.5">Featured App</Badge>
                  <span className="text-xs font-semibold text-muted-foreground flex items-center gap-1">
                    <Star className="h-3 w-3 fill-amber-400 text-amber-400" /> 4.9 (World&#39;s #1 CMS)
                  </span>
                </div>
                <h3 className="text-lg sm:text-xl font-bold tracking-tight">WordPress v6.7.1</h3>
                <p className="text-xs sm:text-sm text-muted-foreground max-w-xl">
                  {t("apps.wpHeroDesc", "World's most popular web publishing platform. Create websites, blogs, and stores with one-click automated setup.")}
                </p>
              </div>
            </div>

            {catalog.find((a) => a.id === "wordpress") && (
              <Button
                onClick={() => onSelectApp(catalog.find((a) => a.id === "wordpress")!)}
                className="shrink-0 font-semibold gap-2 shadow-sm"
              >
                <Download className="h-4 w-4" />
                <span>{t("apps.installNow", "Install WordPress")}</span>
              </Button>
            )}
          </div>
        </div>
      )}

      {/* Search and Filters */}
      <div className="flex flex-col sm:flex-row items-center justify-between gap-3">
        <div className="relative w-full sm:w-80">
          <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
          <Input
            type="text"
            placeholder={t("apps.searchCatalog", "Search applications...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-9 text-xs h-9"
            data-testid="input-search-catalog"
          />
        </div>

        <div className="flex items-center gap-1.5 w-full sm:w-auto overflow-x-auto pb-1 sm:pb-0">
          {categories.map((cat) => (
            <Button
              key={cat.id}
              variant={selectedCategory === cat.id ? "default" : "outline"}
              size="sm"
              onClick={() => setSelectedCategory(cat.id)}
              className={`text-xs whitespace-nowrap h-8 px-3.5 ${
                selectedCategory === cat.id ? "font-bold shadow-xs" : "text-muted-foreground"
              }`}
              data-testid={`btn-filter-cat-${cat.id}`}
            >
              {t(cat.labelKey, cat.defaultLabel)}
            </Button>
          ))}
        </div>
      </div>

      {/* Grid of Apps */}
      {filteredApps.length === 0 ? (
        <div className="p-12 text-center border rounded-xl bg-card" data-testid="empty-catalog-placeholder">
          <p className="text-sm font-semibold">{t("apps.noAppsFound", "No applications match your search.")}</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5" data-testid="catalog-grid">
          {filteredApps.map((app) => {
            const desc = isBn && app.description_bn ? app.description_bn : app.description;
            const logoUrl = APP_LOGOS[app.id.toLowerCase()] || APP_LOGOS[app.icon.toLowerCase()];
            const bannerUrl = APP_BANNERS[app.id.toLowerCase()];

            return (
              <Card
                key={app.id}
                data-testid={`catalog-card-${app.id}`}
                className="flex flex-col justify-between hover:shadow-lg transition-all duration-200 border overflow-hidden group"
              >
                {bannerUrl && (
                  <div className="relative h-28 w-full overflow-hidden bg-muted/40 border-b">
                    <img
                      src={bannerUrl}
                      alt={`${app.name} preview`}
                      className="w-full h-full object-cover object-top opacity-90 group-hover:scale-105 transition-transform duration-300"
                    />
                    <div className="absolute inset-0 bg-gradient-to-t from-background/90 via-background/20 to-transparent" />
                  </div>
                )}

                <CardHeader className="pb-3 pt-4">
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex items-center gap-3 min-w-0">
                      <div className="app-logo-box shrink-0 flex items-center justify-center rounded-xl bg-white dark:bg-card/90 border border-slate-200 dark:border-slate-800 p-2 shadow-xs h-13 w-13 overflow-hidden">
                        {logoUrl ? (
                          <img
                            src={logoUrl}
                            alt={app.name}
                            className="max-h-full max-w-full object-contain pointer-events-none select-none"
                            onError={(e) => {
                              (e.currentTarget as HTMLElement).style.display = "none";
                            }}
                          />
                        ) : (
                          getFallbackIcon(app.icon)
                        )}
                      </div>
                      <div className="min-w-0">
                        <CardTitle className="text-base font-bold flex items-center gap-2">
                          <span data-testid={`app-name-${app.id}`} className="truncate">{app.name}</span>
                        </CardTitle>
                        <CardDescription className="text-xs font-mono">
                          v{app.version}
                        </CardDescription>
                      </div>
                    </div>
                    <Badge variant="outline" className="text-[10px] uppercase font-bold tracking-wider shrink-0 bg-muted/30">
                      {app.category}
                    </Badge>
                  </div>
                </CardHeader>

                <CardContent className="pb-4 flex-1 flex flex-col justify-between">
                  <p className="text-xs text-muted-foreground line-clamp-3 leading-relaxed">
                    {desc}
                  </p>

                  <div className="flex flex-wrap items-center gap-1.5 mt-4 text-[11px] text-muted-foreground">
                    {app.requires_db && (
                      <span className="inline-flex items-center gap-1 bg-muted/60 px-2 py-0.5 rounded-md font-medium text-[11px]">
                        <Database className="h-3 w-3 text-muted-foreground" />
                        {t("apps.requiresDb", "MySQL Required")}
                      </span>
                    )}
                    {app.requires_php && app.min_php_version && (
                      <span className="inline-flex items-center gap-1 bg-muted/60 px-2 py-0.5 rounded-md font-medium text-[11px]">
                        <Code className="h-3 w-3 text-muted-foreground" />
                        PHP {app.min_php_version}+
                      </span>
                    )}
                  </div>
                </CardContent>

                <CardFooter className="pt-3 border-t bg-muted/10 flex items-center justify-between">
                  {app.documentation_url ? (
                    <a
                      href={app.documentation_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-muted-foreground hover:text-primary transition-colors inline-flex items-center gap-1"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                      {t("apps.docs", "Docs")}
                    </a>
                  ) : (
                    <div />
                  )}

                  <Button
                    size="sm"
                    onClick={() => onSelectApp(app)}
                    data-testid={`btn-install-${app.id}`}
                    className="flex items-center gap-1.5 h-8 text-xs font-semibold shadow-xs"
                  >
                    <Download className="h-3.5 w-3.5" />
                    <span>{t("apps.installNow", "Install")}</span>
                  </Button>
                </CardFooter>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
