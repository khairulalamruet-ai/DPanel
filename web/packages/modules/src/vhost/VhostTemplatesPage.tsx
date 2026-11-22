import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { listTemplates, saveTemplate } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  FileCode2,
  Server,
  Layers,
  Save,
  History,
  Send,
  Eye,
  Edit3,
  RefreshCw,
} from "lucide-react";
import type { VhostTemplate } from "./types";
import { VhostEditor } from "./VhostEditor";
import { VhostPreview } from "./VhostPreview";
import { VhostVersionHistoryDialog } from "./VhostVersionHistoryDialog";
import { VhostApplyAllDialog } from "./VhostApplyAllDialog";

export function VhostTemplatesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [templates, setTemplates] = useState<VhostTemplate[]>([]);
  const [selectedTemplateId, setSelectedTemplateId] = useState<string | null>(null);
  const [currentContent, setCurrentContent] = useState<string>("");
  const [activeTab, setActiveTab] = useState<"editor" | "preview">("editor");

  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);

  // Modals
  const [historyOpen, setHistoryOpen] = useState(false);
  const [applyAllOpen, setApplyAllOpen] = useState(false);

  // Load templates list
  const fetchTemplates = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = { items: await listTemplates() };
      setTemplates(res.items);
      if (res.items.length > 0 && !selectedTemplateId) {
        setSelectedTemplateId(res.items[0].id);
        setCurrentContent(res.items[0].content);
      } else if (selectedTemplateId) {
        const found = res.items.find((t) => t.id === selectedTemplateId);
        if (found) setCurrentContent(found.content);
      }
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("vhost.errors.fetchFailed", "Failed to load vhost templates"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [selectedTemplateId, t, toast]);

  useEffect(() => {
    fetchTemplates();
  }, [fetchTemplates]);

  const selectedTemplate = templates.find((t) => t.id === selectedTemplateId) || null;

  const handleSelectTemplate = (tpl: VhostTemplate) => {
    setSelectedTemplateId(tpl.id);
    setCurrentContent(tpl.content);
    setActiveTab("editor");
  };

  const handleSave = async () => {
    if (!selectedTemplate) return;
    try {
      setIsSaving(true);
      const updated = await saveTemplate(selectedTemplate, currentContent);

      setTemplates((prev) =>
        prev.map((t) => (t.id === updated.id ? updated : t))
      );
      setCurrentContent(updated.content);

      toast({
        title: t("common.success", "Success"),
        description: t(
          "vhost.toast.saved",
          "Virtual host template saved successfully (revision v{{version}} created).",
          { version: updated.version }
        ),
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("vhost.errors.saveFailed", "Failed to save vhost template"),
        variant: "destructive",
      });
    } finally {
      setIsSaving(false);
    }
  };

  const hasUnsavedChanges = selectedTemplate ? currentContent !== selectedTemplate.content : false;

  return (
    <div className="space-y-6" data-testid="vhost-templates-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FileCode2 className="h-6 w-6 text-primary" />
            {t("vhost.title", "VHost Templates & Directives")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "vhost.subtitle",
              "Manage Nginx and Caddy server block templates, validate configuration directives, and deploy across domains."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={fetchTemplates}
            disabled={isLoading}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-refresh-vhost"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Main Workspace Layout */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 items-start">
        {/* Left Column: Template Selection Cards */}
        <div className="lg:col-span-4 space-y-3">
          <div className="flex items-center justify-between pb-1">
            <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("vhost.templatesList", "Available Templates")}
            </h2>
            <Badge variant="outline" className="text-[11px] font-mono">
              {templates.length}
            </Badge>
          </div>

          <div className="space-y-2.5">
            {isLoading ? (
              <Card className="p-8 text-center text-xs text-muted-foreground">
                <RefreshCw className="h-5 w-5 animate-spin mx-auto mb-2 text-primary" />
                {t("common.loading", "Loading templates...")}
              </Card>
            ) : (
              templates.map((tpl) => {
                const isSelected = tpl.id === selectedTemplateId;
                return (
                  <Card
                    key={tpl.id}
                    onClick={() => handleSelectTemplate(tpl)}
                    className={`cursor-pointer transition-all border ${
                      isSelected
                        ? "border-primary ring-1 ring-primary/40 bg-primary/[0.02]"
                        : "hover:border-border hover:shadow-sm"
                    }`}
                    data-testid={`card-vhost-template-${tpl.slug}`}
                  >
                    <CardContent className="p-4 space-y-2">
                      <div className="flex items-start justify-between gap-2">
                        <div className="space-y-0.5">
                          <h3 className="font-semibold text-sm text-foreground flex items-center gap-1.5">
                            {tpl.name}
                          </h3>
                          <p className="font-mono text-[11px] text-muted-foreground">
                            {tpl.slug}
                          </p>
                        </div>

                        <Badge
                          variant="secondary"
                          className="font-mono text-[10px] uppercase font-bold"
                        >
                          {tpl.server_type}
                        </Badge>
                      </div>

                      <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-1 border-t">
                        <div className="flex items-center gap-1.5">
                          <Badge variant="outline" className="text-[10px] h-4 font-mono px-1.5">
                            v{tpl.version}
                          </Badge>
                          {tpl.is_default && (
                            <Badge className="bg-primary/10 text-primary border-primary/20 text-[10px] h-4 px-1.5">
                              Default
                            </Badge>
                          )}
                        </div>

                        <span className="flex items-center gap-1 font-medium">
                          <Layers className="h-3 w-3 text-primary" />
                          {tpl.domain_count} domains
                        </span>
                      </div>
                    </CardContent>
                  </Card>
                );
              })
            )}
          </div>
        </div>

        {/* Right Column: Code Editor & Live Preview Workplace */}
        <div className="lg:col-span-8">
          {selectedTemplate ? (
            <Card className="border shadow-sm">
              <CardContent className="p-5 space-y-4">
                {/* Editor Top Bar */}
                <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between pb-3 border-b gap-3">
                  <div>
                    <div className="flex items-center gap-2">
                      <h2 className="font-bold text-base text-foreground font-mono">
                        {selectedTemplate.name}
                      </h2>
                      <Badge variant="outline" className="font-mono text-[10px]">
                        v{selectedTemplate.version}
                      </Badge>
                      {hasUnsavedChanges && (
                        <Badge variant="secondary" className="bg-amber-500/10 text-amber-600 text-[10px]">
                          Unsaved changes
                        </Badge>
                      )}
                    </div>
                    <p className="text-xs text-muted-foreground mt-0.5 font-mono">
                      Target server: {selectedTemplate.server_type.toUpperCase()} • {selectedTemplate.domain_count} active domains
                    </p>
                  </div>

                  {/* Actions & View Switcher */}
                  <div className="flex items-center gap-2 flex-wrap">
                    {/* Editor / Preview Switch */}
                    <div className="flex items-center rounded-md border p-0.5 bg-muted/40">
                      <Button
                        type="button"
                        size="sm"
                        variant={activeTab === "editor" ? "default" : "ghost"}
                        onClick={() => setActiveTab("editor")}
                        className="h-7 text-xs px-2.5 gap-1"
                        data-testid="tab-editor"
                      >
                        <Edit3 className="h-3 w-3" />
                        Editor
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant={activeTab === "preview" ? "default" : "ghost"}
                        onClick={() => setActiveTab("preview")}
                        className="h-7 text-xs px-2.5 gap-1"
                        data-testid="tab-preview"
                      >
                        <Eye className="h-3 w-3" />
                        Preview
                      </Button>
                    </div>

                    {/* Version History Button */}
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => setHistoryOpen(true)}
                      className="h-8 text-xs gap-1.5"
                      data-testid="btn-vhost-history"
                      title="Inspect revisions"
                    >
                      <History className="h-3.5 w-3.5" />
                      History
                    </Button>

                    {/* Apply to All Domains (DoD T06.07) */}
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => setApplyAllOpen(true)}
                      className="h-8 text-xs gap-1.5 text-primary border-primary/30 hover:bg-primary/5"
                      data-testid="btn-apply-all-domains"
                    >
                      <Send className="h-3.5 w-3.5 text-primary" />
                      Apply to All
                    </Button>

                    {/* Save Button */}
                    <Button
                      size="sm"
                      onClick={handleSave}
                      disabled={isSaving}
                      className="h-8 text-xs gap-1.5"
                      data-testid="btn-save-vhost"
                    >
                      <Save className="h-3.5 w-3.5" />
                      {isSaving ? "Saving..." : "Save"}
                    </Button>
                  </div>
                </div>

                {/* Main Component Body */}
                {activeTab === "editor" ? (
                  <VhostEditor
                    value={currentContent}
                    onChange={setCurrentContent}
                    variables={selectedTemplate.variables}
                    serverType={selectedTemplate.server_type}
                  />
                ) : (
                  <VhostPreview content={currentContent} />
                )}
              </CardContent>
            </Card>
          ) : (
            <Card className="p-12 text-center text-xs text-muted-foreground">
              <Server className="h-8 w-8 mx-auto mb-2 text-muted-foreground/40" />
              Select a template to view and edit virtual host directives.
            </Card>
          )}
        </div>
      </div>

      {/* History Dialog */}
      <VhostVersionHistoryDialog
        open={historyOpen}
        onOpenChange={setHistoryOpen}
        template={selectedTemplate}
        onReverted={(revertedTpl) => {
          setTemplates((prev) =>
            prev.map((t) => (t.id === revertedTpl.id ? revertedTpl : t))
          );
          setCurrentContent(revertedTpl.content);
          toast({
            title: t("common.success", "Success"),
            description: t(
              "vhost.toast.reverted",
              "Template restored to revision v{{version}}.",
              { version: revertedTpl.version }
            ),
          });
        }}
      />

      {/* Apply to All Domains Dialog (DoD T06.07) */}
      <VhostApplyAllDialog
        open={applyAllOpen}
        onOpenChange={setApplyAllOpen}
        template={selectedTemplate}
        onSuccess={() => {
          fetchTemplates();
        }}
      />
    </div>
  );
}
