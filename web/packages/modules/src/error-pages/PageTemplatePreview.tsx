import React from "react";
import { Button, Input } from "@builtpanel/ui";
import { Monitor, Tablet, Smartphone, ExternalLink, RefreshCw } from "lucide-react";
import { PageTemplateVariable } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface PageTemplatePreviewProps {
  content: string;
  variables: PageTemplateVariable[];
  title: string;
}

export function PageTemplatePreview({
  content,
  variables,
  title,
}: PageTemplatePreviewProps) {
  const { t } = useTranslation();
  const [device, setDevice] = React.useState<"desktop" | "tablet" | "mobile">("desktop");
  const [customVars, setCustomVars] = React.useState<Record<string, string>>({});
  const [showConfig, setShowConfig] = React.useState(false);

  // Initialize sample values
  React.useEffect(() => {
    const defaults: Record<string, string> = {};
    variables.forEach((v) => {
      defaults[v.name] = v.sample;
    });
    setCustomVars(defaults);
  }, [variables]);

  // Interpolate variables
  const renderedHtml = React.useMemo(() => {
    let result = content;
    variables.forEach((v) => {
      const val = customVars[v.name] ?? v.sample;
      result = result.split(v.name).join(val);
    });
    return result;
  }, [content, variables, customVars]);

  const handleOpenNewTab = () => {
    const blob = new Blob([renderedHtml], { type: "text/html" });
    const url = URL.createObjectURL(blob);
    window.open(url, "_blank");
  };

  const getContainerWidth = () => {
    switch (device) {
      case "mobile":
        return "375px";
      case "tablet":
        return "768px";
      case "desktop":
      default:
        return "100%";
    }
  };

  return (
    <div className="flex flex-col h-full rounded-lg border border-border bg-card overflow-hidden">
      {/* Preview Toolbar */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border bg-muted/40 px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("errorPages.preview.title")}: {title}
          </span>
        </div>

        {/* Device Switcher */}
        <div className="flex items-center gap-1 bg-background/80 rounded-md p-1 border border-border">
          <button
            type="button"
            onClick={() => setDevice("desktop")}
            className={`p-1.5 rounded text-xs transition-colors ${
              device === "desktop"
                ? "bg-primary text-primary-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
            title={t("errorPages.preview.desktop")}
            data-testid="preview-device-desktop"
          >
            <Monitor className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={() => setDevice("tablet")}
            className={`p-1.5 rounded text-xs transition-colors ${
              device === "tablet"
                ? "bg-primary text-primary-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
            title={t("errorPages.preview.tablet")}
            data-testid="preview-device-tablet"
          >
            <Tablet className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={() => setDevice("mobile")}
            className={`p-1.5 rounded text-xs transition-colors ${
              device === "mobile"
                ? "bg-primary text-primary-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
            title={t("errorPages.preview.mobile")}
            data-testid="preview-device-mobile"
          >
            <Smartphone className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowConfig(!showConfig)}
            className="text-xs"
            data-testid="btn-toggle-preview-vars"
          >
            {showConfig ? t("errorPages.preview.hideVars") : t("errorPages.preview.customizeVars")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleOpenNewTab}
            className="text-xs"
            title={t("errorPages.preview.openExternal")}
            data-testid="btn-open-preview-tab"
          >
            <ExternalLink className="h-3.5 w-3.5 mr-1" />
            {t("errorPages.preview.openTab")}
          </Button>
        </div>
      </div>

      {/* Variable Customizer Accordion */}
      {showConfig && (
        <div className="border-b border-border bg-muted/20 px-4 py-3" data-testid="preview-vars-panel">
          <div className="text-xs font-semibold text-foreground mb-2">
            {t("errorPages.preview.previewVariables")}:
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-3">
            {variables.map((v) => (
              <div key={v.name} className="flex flex-col gap-1">
                <label className="text-xs font-mono text-muted-foreground">
                  {v.name}
                </label>
                <Input
                  value={customVars[v.name] ?? v.sample}
                  onChange={(e) =>
                    setCustomVars((prev) => ({ ...prev, [v.name]: e.target.value }))
                  }
                  className="h-8 text-xs font-mono"
                  data-testid={`input-sample-${v.name.replace(/[{}]/g, "")}`}
                />
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Iframe Viewport Container */}
      <div className="flex-1 bg-muted/30 p-4 flex items-center justify-center overflow-auto min-h-[420px]">
        <div
          style={{ width: getContainerWidth() }}
          className="h-full min-h-[400px] transition-all duration-300 rounded-md shadow-md border border-border bg-white overflow-hidden flex flex-col"
        >
          <iframe
            srcDoc={renderedHtml}
            title={title}
            sandbox="allow-same-origin allow-scripts"
            className="w-full h-full flex-1 border-0"
            data-testid="preview-iframe"
          />
        </div>
      </div>
    </div>
  );
}
