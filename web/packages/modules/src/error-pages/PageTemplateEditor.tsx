import React from "react";
import { Badge, Button } from "@builtpanel/ui";
import { Copy, Plus, Check } from "lucide-react";
import { PageTemplateVariable } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface PageTemplateEditorProps {
  content: string;
  onChange: (content: string) => void;
  variables: PageTemplateVariable[];
  isDirty: boolean;
  onSave: () => void;
  onResetClick: () => void;
  isSaving: boolean;
  isCustomized: boolean;
}

export function PageTemplateEditor({
  content,
  onChange,
  variables,
  isDirty,
  onSave,
  onResetClick,
  isSaving,
  isCustomized,
}: PageTemplateEditorProps) {
  const { t } = useTranslation();
  const [copiedVar, setCopiedVar] = React.useState<string | null>(null);
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

  const lines = content.split("\n");
  const lineCount = lines.length;
  const charCount = content.length;

  const handleInsertVariable = (varName: string) => {
    const textarea = textareaRef.current;
    if (!textarea) {
      onChange(content + " " + varName);
      return;
    }
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const newContent = content.substring(0, start) + varName + content.substring(end);
    onChange(newContent);

    // Copy to clipboard as feedback
    navigator.clipboard?.writeText(varName);
    setCopiedVar(varName);
    setTimeout(() => setCopiedVar(null), 1800);

    // Refocus
    setTimeout(() => {
      textarea.focus();
      textarea.setSelectionRange(start + varName.length, start + varName.length);
    }, 50);
  };

  return (
    <div className="flex flex-col h-full rounded-lg border border-border bg-card overflow-hidden">
      {/* Editor Toolbar */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border bg-muted/40 px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("errorPages.editor.htmlSource")}
          </span>
          {isDirty && (
            <Badge variant="secondary" className="text-xs">
              {t("errorPages.editor.unsavedChanges")}
            </Badge>
          )}
          {isCustomized ? (
            <Badge variant="default" className="text-xs">
              {t("errorPages.editor.customized")}
            </Badge>
          ) : (
            <Badge variant="outline" className="text-xs">
              {t("errorPages.editor.defaultTemplate")}
            </Badge>
          )}
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={onResetClick}
            disabled={isSaving}
            data-testid="btn-reset-template"
          >
            {t("errorPages.actions.resetDefault")}
          </Button>
          <Button
            size="sm"
            onClick={onSave}
            disabled={!isDirty || isSaving}
            data-testid="btn-save-template"
          >
            {isSaving ? t("common.saving") : t("errorPages.actions.saveTemplate")}
          </Button>
        </div>
      </div>

      {/* Variables Bar */}
      <div className="border-b border-border bg-muted/20 px-4 py-2">
        <div className="flex items-center gap-2 mb-1.5">
          <span className="text-xs font-medium text-foreground">
            {t("errorPages.editor.availableVariables")}:
          </span>
          <span className="text-xs text-muted-foreground">
            ({t("errorPages.editor.clickToInsert")})
          </span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {variables.map((v) => (
            <button
              key={v.name}
              type="button"
              onClick={() => handleInsertVariable(v.name)}
              className="group inline-flex items-center gap-1 rounded bg-secondary px-2 py-0.5 text-xs font-mono text-secondary-foreground hover:bg-primary/20 hover:text-primary transition-colors border border-border"
              title={`${v.description} (Sample: ${v.sample})`}
              data-testid={`var-chip-${v.name.replace(/[{}]/g, "")}`}
            >
              <span>{v.name}</span>
              {copiedVar === v.name ? (
                <Check className="h-3 w-3 text-emerald-500" />
              ) : (
                <Plus className="h-3 w-3 opacity-60 group-hover:opacity-100" />
              )}
            </button>
          ))}
        </div>
      </div>

      {/* Code Textarea with line numbers */}
      <div className="flex-1 flex overflow-hidden min-h-[420px]">
        {/* Line Numbers Gutter */}
        <div
          className="select-none bg-muted/30 px-3 py-3 text-right font-mono text-xs text-muted-foreground/60 border-r border-border overflow-hidden"
          style={{ width: "4rem" }}
          aria-hidden="true"
        >
          {Array.from({ length: Math.min(lineCount, 600) }).map((_, i) => (
            <div key={i + 1} className="leading-6">
              {i + 1}
            </div>
          ))}
        </div>

        {/* Textarea */}
        <textarea
          ref={textareaRef}
          value={content}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          className="flex-1 resize-none bg-background p-3 font-mono text-xs leading-6 text-foreground focus:outline-none focus:ring-0 overflow-auto"
          aria-label={t("errorPages.editor.htmlSource")}
          data-testid="textarea-template-content"
        />
      </div>

      {/* Editor Status Bar */}
      <div className="flex items-center justify-between border-t border-border bg-muted/40 px-4 py-1.5 text-xs text-muted-foreground font-mono">
        <div>HTML5 Document</div>
        <div className="flex items-center gap-4">
          <span>{lineCount} lines</span>
          <span>{charCount} characters</span>
        </div>
      </div>
    </div>
  );
}
