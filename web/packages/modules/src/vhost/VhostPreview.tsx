import React, { useState, useMemo, useEffect } from "react";
import { previewTemplate } from "./api";
import { Input, Button } from "@builtpanel/ui";
import { Eye, SlidersHorizontal, RotateCcw } from "lucide-react";

interface VhostPreviewProps {
  content: string;
}

const defaultVariables = {
  domain: "my-awesome-site.com",
  docroot: "/home/demo/public_html",
  php_socket: "/run/php/php8.3-fpm-demo.sock",
  port: "3000",
  ssl_cert: "/etc/ssl/certs/my-awesome-site.com.crt",
  ssl_key: "/etc/ssl/private/my-awesome-site.com.key",
};

export function VhostPreview({ content }: VhostPreviewProps) {
  const [variables, setVariables] = useState<Record<string, string>>(defaultVariables);
  const [showConfig, setShowConfig] = useState(false);

  // Render variables into content
  const renderedContent = useMemo(() => {
    let result = content;
    for (const [key, val] of Object.entries(variables)) {
      const reg = new RegExp(`\\{\\{\\s*${key}\\s*\\}\\}`, "g");
      result = result.replace(reg, val);
    }
    return result;
  }, [content, variables]);

  // What the server makes of it (a site that uses every part); until it answers, or when it
  // cannot, the text is filled in here with the sample variables.
  const [serverRendered, setServerRendered] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      previewTemplate(content)
        .then((r) => live && setServerRendered(r))
        .catch(() => live && setServerRendered(null));
    }, 300);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [content]);

  const handleResetVars = () => {
    setVariables(defaultVariables);
  };

  const updateVariable = (key: string, val: string) => {
    setVariables((prev) => ({ ...prev, [key]: val }));
  };

  return (
    <div className="space-y-3" data-testid="vhost-preview">
      {/* Toolbar */}
      <div className="flex items-center justify-between p-2.5 bg-muted/40 rounded-lg border text-xs">
        <div className="flex items-center gap-1.5 font-medium text-muted-foreground">
          <Eye className="h-4 w-4 text-primary" />
          <span>Interpolated Live Output</span>
        </div>

        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setShowConfig((prev) => !prev)}
            className="h-7 text-xs gap-1.5"
            data-testid="btn-toggle-sample-vars"
          >
            <SlidersHorizontal className="h-3 w-3" />
            {showConfig ? "Hide Sample Variables" : "Sample Variables"}
          </Button>

          {showConfig && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleResetVars}
              className="h-7 text-xs gap-1"
              title="Reset sample parameters"
            >
              <RotateCcw className="h-3 w-3" />
              Reset
            </Button>
          )}
        </div>
      </div>

      {/* Sample Variables Panel */}
      {showConfig && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2.5 p-3 bg-card border rounded-lg text-xs">
          <div>
            <label className="block text-[11px] font-semibold text-muted-foreground mb-1">
              {"{{domain}}"}
            </label>
            <Input
              value={variables.domain}
              onChange={(e) => updateVariable("domain", e.target.value)}
              className="h-8 text-xs font-mono"
              data-testid="input-sample-domain"
            />
          </div>

          <div>
            <label className="block text-[11px] font-semibold text-muted-foreground mb-1">
              {"{{docroot}}"}
            </label>
            <Input
              value={variables.docroot}
              onChange={(e) => updateVariable("docroot", e.target.value)}
              className="h-8 text-xs font-mono"
              data-testid="input-sample-docroot"
            />
          </div>

          <div>
            <label className="block text-[11px] font-semibold text-muted-foreground mb-1">
              {"{{php_socket}}"}
            </label>
            <Input
              value={variables.php_socket}
              onChange={(e) => updateVariable("php_socket", e.target.value)}
              className="h-8 text-xs font-mono"
            />
          </div>
        </div>
      )}

      {/* Rendered output display */}
      <div className="border rounded-lg bg-zinc-950 font-mono text-xs overflow-hidden shadow-inner">
        <pre
          className="p-4 text-emerald-400 font-mono text-[12px] leading-5 overflow-x-auto selection:bg-emerald-800/50"
          data-testid="pre-vhost-rendered"
        >
          {serverRendered ?? renderedContent}
        </pre>
      </div>
    </div>
  );
}
