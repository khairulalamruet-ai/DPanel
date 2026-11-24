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
} from "lucide-react";
import { CatalogApp, AppCategory } from "./types";

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

  const getAppIcon = (iconName: string) => {
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
    <div className="space-y-4" data-testid="catalog-grid-container">
      {/* Search and Filters */}
      <div className="flex flex-col sm:flex-row items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder={t("apps.searchCatalog", "Search applications...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8 text-xs"
            data-testid="input-search-catalog"
          />
        </div>

        <div className="flex items-center gap-1.5 w-full sm:w-auto overflow-x-auto pb-1 sm:pb-0">
          {categories.map((cat) => (
            <Button
              key={cat.id}
              variant={selectedCategory === cat.id ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setSelectedCategory(cat.id)}
              className={`text-xs whitespace-nowrap h-8 px-3 ${
                selectedCategory === cat.id ? "font-bold" : "text-muted-foreground"
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
        <div className="p-8 text-center border rounded-lg bg-card" data-testid="empty-catalog-placeholder">
          <p className="text-sm font-semibold">{t("apps.noAppsFound", "No applications match your search.")}</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4" data-testid="catalog-grid">
          {filteredApps.map((app) => {
            const desc = isBn && app.description_bn ? app.description_bn : app.description;

            return (
              <Card
                key={app.id}
                data-testid={`catalog-card-${app.id}`}
                className="flex flex-col justify-between hover:shadow-md transition-shadow border"
              >
                <CardHeader className="pb-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex items-center gap-3">
                      <div className="p-2.5 bg-muted rounded-xl border flex items-center justify-center">
                        {getAppIcon(app.icon)}
                      </div>
                      <div>
                        <CardTitle className="text-base font-bold flex items-center gap-2">
                          <span data-testid={`app-name-${app.id}`}>{app.name}</span>
                        </CardTitle>
                        <CardDescription className="text-xs font-mono">
                          v{app.version}
                        </CardDescription>
                      </div>
                    </div>
                    <Badge variant="outline" className="text-[10px] uppercase font-bold tracking-wider">
                      {app.category}
                    </Badge>
                  </div>
                </CardHeader>

                <CardContent className="pb-4 flex-1">
                  <p className="text-xs text-muted-foreground line-clamp-3 leading-relaxed">
                    {desc}
                  </p>

                  <div className="flex flex-wrap items-center gap-2 mt-4 text-[11px] text-muted-foreground">
                    {app.requires_db && (
                      <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded font-medium">
                        <Database className="h-3 w-3 text-muted-foreground" />
                        {t("apps.requiresDb", "MySQL Required")}
                      </span>
                    )}
                    {app.requires_php && app.min_php_version && (
                      <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded font-medium">
                        <Code className="h-3 w-3 text-muted-foreground" />
                        PHP {app.min_php_version}+
                      </span>
                    )}
                  </div>
                </CardContent>

                <CardFooter className="pt-2 border-t flex items-center justify-between">
                  {app.documentation_url ? (
                    <a
                      href={app.documentation_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-muted-foreground hover:text-primary inline-flex items-center gap-1"
                    >
                      <ExternalLink className="h-3 w-3" />
                      {t("apps.docs", "Docs")}
                    </a>
                  ) : (
                    <div />
                  )}

                  <Button
                    size="sm"
                    onClick={() => onSelectApp(app)}
                    data-testid={`btn-install-${app.id}`}
                    className="flex items-center gap-1.5 h-8 text-xs font-semibold"
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
