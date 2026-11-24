import React, { useMemo } from "react";
import { Button } from "@builtpanel/ui";
import { AlertCircle, CheckCircle2, Code2, Plus } from "lucide-react";

interface VhostEditorProps {
  value: string;
  onChange: (val: string) => void;
  variables: string[];
  serverType: "nginx" | "caddy";
}

export function VhostEditor({ value, onChange, variables, serverType }: VhostEditorProps) {
  // Line numbers calculation
  const linesCount = useMemo(() => {
    return Math.max(1, value.split("\n").length);
  }, [value]);

  // Client-side syntax & security validation
  const validation = useMemo(() => {
    const errors: string[] = [];
    const warnings: string[] = [];

    // Check bracket balance
    const openBraces = (value.match(/\{/g) || []).length;
    const closeBraces = (value.match(/\}/g) || []).length;
    if (openBraces !== closeBraces) {
      errors.push(`Mismatched curly braces: ${openBraces} opening vs ${closeBraces} closing.`);
    }

    // Check essential variables
    if (!value.includes("{{domain}}")) {
      warnings.push("Missing recommended {{domain}} variable tag.");
    }
    if (serverType === "nginx" && !value.includes("{{docroot}}") && !value.includes("proxy_pass")) {
      warnings.push("Nginx vhost usually requires {{docroot}} or proxy_pass directive.");
    }

    // Check forbidden directives (T06.06 security rule)
    if (/include\s+\//i.test(value)) {
      errors.push("Forbidden directive: 'include /' root directory inclusion is restricted.");
    }
    if (/alias\s+\/etc/i.test(value)) {
      errors.push("Forbidden directive: 'alias /etc' system directory mapping is restricted.");
    }

    return {
      isValid: errors.length === 0,
      errors,
      warnings,
    };
  }, [value, serverType]);

  const insertVariable = (varName: string) => {
    const token = `{{${varName}}}`;
    onChange(value + ` ${token} `);
  };

  return (
    <div className="space-y-3" data-testid="vhost-editor">
      {/* Variable quick-insert helper chips */}
      <div className="flex flex-wrap items-center gap-2 p-2.5 bg-muted/50 rounded-lg border text-xs">
        <span className="font-semibold text-muted-foreground flex items-center gap-1.5">
          <Code2 className="h-3.5 w-3.5 text-primary" />
          Template Variables:
        </span>
        {variables.map((v) => (
          <Button
            key={v}
            type="button"
            variant="outline"
            size="sm"
            onClick={() => insertVariable(v)}
            className="h-6 text-[11px] px-2 py-0 font-mono gap-1 hover:bg-primary/10 hover:border-primary/40"
            title={`Insert {{${v}}}`}
            data-testid={`btn-insert-var-${v}`}
          >
            <Plus className="h-2.5 w-2.5 text-primary" />
            {`{{${v}}}`}
          </Button>
        ))}
      </div>

      {/* Editor container with line numbers */}
      <div className="relative border rounded-lg bg-zinc-950 font-mono text-xs overflow-hidden shadow-inner flex">
        {/* Line numbers gutter */}
        <div
          className="select-none py-3 px-3 text-right text-zinc-600 bg-zinc-900/60 border-r border-zinc-800 text-[11px] leading-5 font-mono"
          aria-hidden="true"
        >
          {Array.from({ length: linesCount }).map((_, idx) => (
            <div key={idx + 1}>{idx + 1}</div>
          ))}
        </div>

        {/* Code textarea */}
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          className="w-full h-[360px] p-3 bg-transparent text-zinc-100 font-mono text-[12px] leading-5 resize-none focus:outline-none focus:ring-1 focus:ring-primary/50 selection:bg-primary/30"
          data-testid="textarea-vhost-content"
          placeholder="Enter virtual host configuration template..."
        />
      </div>

      {/* Real-time Syntax / Security Status Banner */}
      <div className="flex flex-col gap-2">
        {validation.errors.length > 0 && (
          <div
            className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg space-y-1"
            data-testid="vhost-validation-errors"
          >
            <div className="flex items-center gap-1.5 font-semibold">
              <AlertCircle className="h-4 w-4 shrink-0" />
              Syntax / Security Errors:
            </div>
            <ul className="list-disc pl-5 space-y-0.5">
              {validation.errors.map((err, i) => (
                <li key={i}>{err}</li>
              ))}
            </ul>
          </div>
        )}

        {validation.warnings.length > 0 && (
          <div
            className="p-2.5 text-xs bg-amber-500/10 border border-amber-500/20 text-amber-700 dark:text-amber-400 rounded-lg space-y-1"
            data-testid="vhost-validation-warnings"
          >
            <div className="flex items-center gap-1.5 font-semibold">
              <AlertCircle className="h-3.5 w-3.5 shrink-0" />
              Suggestions:
            </div>
            <ul className="list-disc pl-5 space-y-0.5">
              {validation.warnings.map((warn, i) => (
                <li key={i}>{warn}</li>
              ))}
            </ul>
          </div>
        )}

        {validation.isValid && validation.warnings.length === 0 && (
          <div className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400 font-medium">
            <CheckCircle2 className="h-3.5 w-3.5" />
            Template syntax is well-formed.
          </div>
        )}
      </div>
    </div>
  );
}
