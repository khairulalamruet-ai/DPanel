import React, { useState, useEffect, useCallback } from "react";
import { listPages, savePage, resetPage } from "./api";
import { Badge, Button, useToast } from "@builtpanel/ui";
import {
  Globe,
  UserX,
  FileQuestion,
  ServerCrash,
  Columns,
  Maximize2,
} from "lucide-react";
import { PageTemplateItem, PageTemplateType } from "./types";
import { PageTemplateEditor } from "./PageTemplateEditor";
import { PageTemplatePreview } from "./PageTemplatePreview";
import { PageTemplateResetDialog } from "./PageTemplateResetDialog";
import { useTranslation } from "@builtpanel/i18n";

export function ErrorPagesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeType, setActiveType] = useState<PageTemplateType>("suspended_domain");
  const [viewMode, setViewMode] = useState<"split" | "tabs">("split");
  const [activeSubTab, setActiveSubTab] = useState<"editor" | "preview">("editor");
  const [templates, setTemplates] = useState<PageTemplateItem[]>([]);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isResetting, setIsResetting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resetDialogOpen, setResetDialogOpen] = useState(false);

  // 1. Fetch templates
  const fetchTemplates = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await listPages();
      setTemplates(res.items);
      const initialDrafts: Record<string, string> = {};
      res.items.forEach((item: PageTemplateItem) => {
        initialDrafts[item.type] = item.content;
      });
      setDrafts(initialDrafts);
    } catch (err: any) {
      setError(err?.message || "Failed to load error page templates");
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchTemplates();
  }, [fetchTemplates]);

  const currentTemplate = templates.find((tpl: PageTemplateItem) => tpl.type === activeType);

  const activeContent =
    currentTemplate && drafts[currentTemplate.type] !== undefined
      ? drafts[currentTemplate.type]
      : currentTemplate?.content || "";

  const isDirty = currentTemplate ? activeContent !== currentTemplate.content : false;

  const handleContentChange = (val: string) => {
    if (!currentTemplate) return;
    setDrafts((prev) => ({ ...prev, [currentTemplate.type]: val }));
  };

  // 2. Save mutation
  const handleSave = async () => {
    if (!currentTemplate) return;
    try {
      setIsSaving(true);
      const updated = await savePage(currentTemplate.type, activeContent);
      setTemplates((prev) =>
        prev.map((item: PageTemplateItem) => (item.type === updated.type ? updated : item))
      );
      setDrafts((prev) => ({ ...prev, [updated.type]: updated.content }));
      toast({
        title: t("common.success"),
        description: t("errorPages.toast.templateSaved", { name: updated.title }),
        variant: "default",
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("errorPages.errors.saveFailed"),
        variant: "destructive",
      });
    } finally {
      setIsSaving(false);
    }
  };

  // 3. Reset mutation
  const handleConfirmReset = async () => {
    if (!currentTemplate) return;
    try {
      setIsResetting(true);
      const updated = await resetPage(currentTemplate.type);
      setTemplates((prev) =>
        prev.map((item: PageTemplateItem) => (item.type === updated.type ? updated : item))
      );
      setDrafts((prev) => ({ ...prev, [updated.type]: updated.content }));
      setResetDialogOpen(false);
      toast({
        title: t("common.success"),
        description: t("errorPages.toast.templateReset", { name: updated.title }),
        variant: "default",
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("errorPages.errors.resetFailed"),
        variant: "destructive",
      });
    } finally {
      setIsResetting(false);
    }
  };

  const getTemplateIcon = (type: PageTemplateType) => {
    switch (type) {
      case "suspended_domain":
        return <Globe className="h-4 w-4" />;
      case "suspended_user":
        return <UserX className="h-4 w-4" />;
      case "error_404":
        return <FileQuestion className="h-4 w-4" />;
      case "error_502":
        return <ServerCrash className="h-4 w-4" />;
    }
  };

  if (isLoading) {
    return (
      <div className="flex h-96 items-center justify-center">
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      </div>
    );
  }

  if (error || !currentTemplate) {
    return (
      <div className="rounded-lg border border-destructive/20 bg-destructive/10 p-6 text-center">
        <p className="text-sm text-destructive">{t("errorPages.errors.fetchFailed")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground" data-testid="error-pages-title">
            {t("errorPages.title")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("errorPages.subtitle")}
          </p>
        </div>

        {/* View Layout Controls */}
        <div className="flex items-center gap-2">
          <Button
            variant={viewMode === "split" ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setViewMode("split")}
            className="hidden lg:inline-flex text-xs"
            title={t("errorPages.layout.splitView")}
            data-testid="btn-view-split"
          >
            <Columns className="h-3.5 w-3.5 mr-1" />
            {t("errorPages.layout.split")}
          </Button>
          <Button
            variant={viewMode === "tabs" ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setViewMode("tabs")}
            className="hidden lg:inline-flex text-xs"
            title={t("errorPages.layout.tabbedView")}
            data-testid="btn-view-tabs"
          >
            <Maximize2 className="h-3.5 w-3.5 mr-1" />
            {t("errorPages.layout.tabs")}
          </Button>
        </div>
      </div>

      {/* Main Template Tabs */}
      <div className="flex flex-wrap border-b border-border gap-2" role="tablist">
        {templates.map((tpl: PageTemplateItem) => {
          const isActive = tpl.type === activeType;
          return (
            <button
              key={tpl.type}
              type="button"
              role="tab"
              aria-selected={isActive}
              onClick={() => setActiveType(tpl.type)}
              className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${
                isActive
                  ? "border-primary text-primary"
                  : "border-transparent text-muted-foreground hover:text-foreground hover:border-border"
              }`}
              data-testid={`tab-template-${tpl.type}`}
            >
              {getTemplateIcon(tpl.type)}
              <span>{tpl.title}</span>
              <span className="text-xs px-1.5 py-0.5 rounded bg-muted font-mono">
                {tpl.statusCode}
              </span>
              {tpl.isCustomized && (
                <span className="h-2 w-2 rounded-full bg-primary" title={t("errorPages.editor.customized")} />
              )}
            </button>
          );
        })}
      </div>

      {/* Mobile/Tablet Sub-tabs when not in split view */}
      {viewMode === "tabs" && (
        <div className="flex items-center gap-2 border-b border-border pb-2">
          <Button
            variant={activeSubTab === "editor" ? "default" : "outline"}
            size="sm"
            onClick={() => setActiveSubTab("editor")}
            data-testid="subtab-editor"
          >
            {t("errorPages.layout.editor")}
          </Button>
          <Button
            variant={activeSubTab === "preview" ? "default" : "outline"}
            size="sm"
            onClick={() => setActiveSubTab("preview")}
            data-testid="subtab-preview"
          >
            {t("errorPages.layout.livePreview")}
          </Button>
        </div>
      )}

      {/* Workspace Area: Split view or Tabbed */}
      {viewMode === "split" ? (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 items-stretch">
          <div className="h-[620px]">
            <PageTemplateEditor
              content={activeContent}
              onChange={handleContentChange}
              variables={currentTemplate.variables}
              isDirty={isDirty}
              onSave={handleSave}
              onResetClick={() => setResetDialogOpen(true)}
              isSaving={isSaving}
              isCustomized={currentTemplate.isCustomized}
            />
          </div>
          <div className="h-[620px]">
            <PageTemplatePreview
              content={activeContent}
              variables={currentTemplate.variables}
              title={currentTemplate.title}
            />
          </div>
        </div>
      ) : activeSubTab === "editor" ? (
        <div className="h-[620px]">
          <PageTemplateEditor
            content={activeContent}
            onChange={handleContentChange}
            variables={currentTemplate.variables}
            isDirty={isDirty}
            onSave={handleSave}
            onResetClick={() => setResetDialogOpen(true)}
            isSaving={isSaving}
            isCustomized={currentTemplate.isCustomized}
          />
        </div>
      ) : (
        <div className="h-[620px]">
          <PageTemplatePreview
            content={activeContent}
            variables={currentTemplate.variables}
            title={currentTemplate.title}
          />
        </div>
      )}

      {/* Reset Confirmation Dialog */}
      <PageTemplateResetDialog
        open={resetDialogOpen}
        onOpenChange={setResetDialogOpen}
        templateTitle={currentTemplate.title}
        onConfirm={handleConfirmReset}
        isLoading={isResetting}
      />
    </div>
  );
}
