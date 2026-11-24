import React, { useEffect, useState, useMemo } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Badge,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Boxes,
  Search,
  Lock,
  Power,
  PowerOff,
  CheckCircle2,
  XCircle,
  RefreshCw,
  GitFork,
} from "lucide-react";
import { ModuleItem, ModuleCategory } from "./types";
import { listModules, setModuleEnabled } from "./api";
import { DependencyWarningDialog } from "./DependencyWarningDialog";

export function ModuleManagerPage() {
  const { t, i18n } = useTranslation();
  const { toast } = useToast();
  const lang = (i18n.language?.startsWith("bn") ? "bn" : "en") as "en" | "bn";

  const [modules, setModules] = useState<ModuleItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Filters
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<ModuleCategory>("all");

  // Dependency Warning Dialog state
  const [warningTarget, setWarningTarget] = useState<ModuleItem | null>(null);
  const [activeDependents, setActiveDependents] = useState<ModuleItem[]>([]);
  const [isWarningOpen, setIsWarningOpen] = useState(false);
  const [isToggling, setIsToggling] = useState<string | null>(null);

  const fetchModules = async () => {
    try {
      setIsLoading(true);
      setError(null);
      setModules(await listModules());
    } catch (err: any) {
      setError(err?.message || "Failed to load modules");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchModules();
  }, []);

  // Compute active dependents of a given module
  const getActiveDependents = (moduleName: string): ModuleItem[] => {
    return modules.filter(
      (m) => m.enabled && m.depends_on && m.depends_on.includes(moduleName)
    );
  };

  const handleToggleClick = (mod: ModuleItem) => {
    if (mod.core) return;

    if (mod.enabled) {
      // Disabling: check if any active module depends on this module
      const dependents = getActiveDependents(mod.name);
      if (dependents.length > 0) {
        setWarningTarget(mod);
        setActiveDependents(dependents);
        setIsWarningOpen(true);
        return;
      }
      // No dependents, disable directly
      executeToggle(mod, false);
    } else {
      // Enabling
      executeToggle(mod, false);
    }
  };

  const executeToggle = async (mod: ModuleItem, cascade: boolean) => {
    try {
      setIsToggling(mod.name);
      // `enabled: true` when the module is off now; the other direction is `cascade` aware.
      await setModuleEnabled(mod.name, !mod.enabled, cascade);
      const modTitle = mod.title[lang] || mod.title.en || mod.name;
      toast({
        title: modTitle,
        description: mod.enabled
          ? t("modules.toggle.disabledSuccess", { name: modTitle })
          : t("modules.toggle.enabledSuccess", { name: modTitle }),
        variant: "default",
      });
      // What changed with it (the modules that depend on it) is read again.
      await fetchModules();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to toggle module",
        variant: "destructive",
      });
    } finally {
      setIsToggling(null);
      setIsWarningOpen(false);
      setWarningTarget(null);
      setActiveDependents([]);
    }
  };

  const handleConfirmWarning = () => {
    if (warningTarget) {
      executeToggle(warningTarget, true);
    }
  };

  // Filtered modules
  const filteredModules = useMemo(() => {
    return modules.filter((mod) => {
      const matchesCategory =
        selectedCategory === "all" || mod.category === selectedCategory;

      const q = searchQuery.toLowerCase().trim();
      if (!q) return matchesCategory;

      const titleEn = mod.title?.en?.toLowerCase() || "";
      const titleBn = mod.title?.bn?.toLowerCase() || "";
      const descEn = mod.description?.en?.toLowerCase() || "";
      const descBn = mod.description?.bn?.toLowerCase() || "";
      const name = mod.name.toLowerCase();

      const matchesSearch =
        name.includes(q) ||
        titleEn.includes(q) ||
        titleBn.includes(q) ||
        descEn.includes(q) ||
        descBn.includes(q);

      return matchesCategory && matchesSearch;
    });
  }, [modules, selectedCategory, searchQuery]);

  const categories: { key: ModuleCategory; label: string }[] = [
    { key: "all", label: t("modules.categories.all") },
    { key: "core", label: t("modules.categories.core") },
    { key: "web", label: t("modules.categories.web") },
    { key: "db", label: t("modules.categories.db") },
    { key: "files", label: t("modules.categories.files") },
    { key: "mail", label: t("modules.categories.mail") },
    { key: "security", label: t("modules.categories.security") },
    { key: "system", label: t("modules.categories.system") },
  ];

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Boxes className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("modules.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("modules.desc")}</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={fetchModules}
          disabled={isLoading}
          className="self-start sm:self-auto gap-2"
        >
          <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          {t("common.reload")}
        </Button>
      </div>

      {/* Main Card */}
      <Card>
        <CardHeader className="space-y-4 pb-4">
          {/* Search & Categories */}
          <div className="flex flex-col md:flex-row gap-3 items-stretch md:items-center justify-between">
            {/* Search Input */}
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder={t("modules.searchPlaceholder")}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-9"
              />
            </div>

            {/* Category Filter Pills */}
            <div className="flex flex-wrap gap-1.5 items-center">
              {categories.map((cat) => (
                <button
                  key={cat.key}
                  type="button"
                  onClick={() => setSelectedCategory(cat.key)}
                  className={`text-xs px-2.5 py-1 rounded-full font-medium transition-colors ${
                    selectedCategory === cat.key
                      ? "bg-primary text-primary-foreground shadow-sm"
                      : "bg-muted/60 text-muted-foreground hover:bg-muted hover:text-foreground"
                  }`}
                >
                  {cat.label}
                </button>
              ))}
            </div>
          </div>
        </CardHeader>

        <CardContent className="p-0">
          {isLoading ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
              {t("common.loading")}
            </div>
          ) : error ? (
            <div className="py-12 text-center space-y-3">
              <p className="text-sm text-destructive">{error}</p>
              <Button variant="outline" size="sm" onClick={fetchModules}>
                {t("common.retry")}
              </Button>
            </div>
          ) : filteredModules.length === 0 ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              {t("modules.empty")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[30%]">{t("modules.table.name")}</TableHead>
                    <TableHead className="w-[12%]">{t("modules.table.category")}</TableHead>
                    <TableHead className="w-[12%]">{t("modules.table.version")}</TableHead>
                    <TableHead className="w-[20%]">{t("modules.table.dependencies")}</TableHead>
                    <TableHead className="w-[12%]">{t("modules.table.status")}</TableHead>
                    <TableHead className="w-[14%] text-right">
                      {t("modules.table.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredModules.map((mod) => {
                    const isBusy = isToggling === mod.name;
                    const titleText = mod.title[lang] || mod.title.en || mod.name;
                    const descText = mod.description[lang] || mod.description.en || "";

                    return (
                      <TableRow key={mod.name} className="hover:bg-muted/50 transition-colors">
                        {/* Name & Title */}
                        <TableCell>
                          <div className="flex flex-col">
                            <div className="flex items-center gap-2">
                              <span className="font-semibold text-foreground text-sm">
                                {titleText}
                              </span>
                              <span className="font-mono text-xs text-muted-foreground">
                                ({mod.name})
                              </span>
                              {mod.core && (
                                <Badge
                                  variant="secondary"
                                  className="text-[10px] px-1.5 py-0 font-normal bg-muted border text-muted-foreground flex items-center gap-1"
                                  title={t("modules.coreTooltip")}
                                >
                                  <Lock className="h-2.5 w-2.5" />
                                  {t("modules.table.coreBadge")}
                                </Badge>
                              )}
                            </div>
                            {descText && (
                              <p className="text-xs text-muted-foreground line-clamp-1 mt-0.5">
                                {descText}
                              </p>
                            )}
                          </div>
                        </TableCell>

                        {/* Category */}
                        <TableCell>
                          <Badge variant="outline" className="capitalize text-xs font-normal">
                            {mod.category}
                          </Badge>
                        </TableCell>

                        {/* Version */}
                        <TableCell>
                          <span className="font-mono text-xs text-muted-foreground">
                            v{mod.version}
                          </span>
                        </TableCell>

                        {/* Dependencies */}
                        <TableCell>
                          {mod.depends_on && mod.depends_on.length > 0 ? (
                            <div className="flex flex-wrap gap-1">
                              {mod.depends_on.map((dep) => (
                                <Badge
                                  key={dep}
                                  variant="secondary"
                                  className="text-[11px] font-mono px-1.5 py-0 flex items-center gap-1"
                                >
                                  <GitFork className="h-2.5 w-2.5 text-muted-foreground" />
                                  {dep}
                                </Badge>
                              ))}
                            </div>
                          ) : (
                            <span className="text-xs text-muted-foreground">
                              {t("modules.table.noDependencies")}
                            </span>
                          )}
                        </TableCell>

                        {/* Status */}
                        <TableCell>
                          {mod.enabled ? (
                            <Badge
                              variant="default"
                              className="text-xs font-medium gap-1 bg-emerald-600 hover:bg-emerald-600/90 text-white"
                            >
                              <CheckCircle2 className="h-3 w-3" />
                              {t("modules.table.active")}
                            </Badge>
                          ) : (
                            <Badge variant="secondary" className="text-xs font-medium gap-1 text-muted-foreground">
                              <XCircle className="h-3 w-3" />
                              {t("modules.table.disabled")}
                            </Badge>
                          )}
                        </TableCell>

                        {/* Action Toggle */}
                        <TableCell className="text-right">
                          {mod.core ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled
                              title={t("modules.coreTooltip")}
                              className="text-xs opacity-50 cursor-not-allowed gap-1.5"
                            >
                              <Lock className="h-3.5 w-3.5" />
                              {t("modules.table.coreBadge")}
                            </Button>
                          ) : mod.enabled ? (
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => handleToggleClick(mod)}
                              disabled={isBusy}
                              className="text-xs gap-1.5 border-destructive/30 text-destructive hover:bg-destructive/10 hover:text-destructive"
                            >
                              <PowerOff className={`h-3.5 w-3.5 ${isBusy ? "animate-spin" : ""}`} />
                              {isBusy
                                ? t("modules.toggle.disabling")
                                : t("modules.toggle.disable")}
                            </Button>
                          ) : (
                            <Button
                              variant="default"
                              size="sm"
                              onClick={() => handleToggleClick(mod)}
                              disabled={isBusy}
                              className="text-xs gap-1.5"
                            >
                              <Power className={`h-3.5 w-3.5 ${isBusy ? "animate-spin" : ""}`} />
                              {isBusy
                                ? t("modules.toggle.enabling")
                                : t("modules.toggle.enable")}
                            </Button>
                          )}
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Dependency Warning Dialog */}
      <DependencyWarningDialog
        open={isWarningOpen}
        onOpenChange={setIsWarningOpen}
        targetModule={warningTarget}
        activeDependents={activeDependents}
        onConfirm={handleConfirmWarning}
        isConfirming={isToggling !== null}
      />
    </div>
  );
}
