import React, { useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, Badge, Button, Input } from "@builtpanel/ui";
import { ShieldCheck, ShieldAlert, ShieldX, Plus, X } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import type { Effective } from "./api";

interface PolicyEditorProps {
  title: string;
  effective: Effective;
  /** Whether a policy of this level is set (it can then be removed). */
  own: boolean;
  onSave: (mode: "detect" | "block", disabledRules: number[]) => Promise<void>;
  onClear?: () => Promise<void>;
}

const FROM: Record<string, string> = {
  domain: "this domain's own policy",
  tenant: "the account's policy",
  global: "the panel's policy",
  default: "the default",
};

/** The firewall mode (off / detect / block) and the rules switched off, for one level of policy. */
export function PolicyEditor({ title, effective, own, onSave, onClear }: PolicyEditorProps) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"detect" | "block">(effective.mode === "block" ? "block" : "detect");
  const [rules, setRules] = useState<number[]>(effective.disabled_rules);
  const [ruleInput, setRuleInput] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setMode(effective.mode === "block" ? "block" : "detect");
    setRules(effective.disabled_rules);
  }, [effective]);

  const run = async (fn: () => Promise<void>) => {
    try {
      setBusy(true);
      await fn();
    } finally {
      setBusy(false);
    }
  };

  const addRule = () => {
    const id = Number(ruleInput.trim());
    if (Number.isInteger(id) && id > 0 && !rules.includes(id)) setRules([...rules, id]);
    setRuleInput("");
  };

  const badge =
    effective.mode === "block" ? (
      <Badge className="gap-1 bg-emerald-600 text-white" data-testid="waf-mode-badge">
        <ShieldCheck className="h-3 w-3" /> {t("waf.modeBlock", "Blocking")}
      </Badge>
    ) : effective.mode === "detect" ? (
      <Badge className="gap-1 bg-amber-500 text-white" data-testid="waf-mode-badge">
        <ShieldAlert className="h-3 w-3" /> {t("waf.modeDetect", "Detect only")}
      </Badge>
    ) : (
      <Badge variant="secondary" className="gap-1" data-testid="waf-mode-badge">
        <ShieldX className="h-3 w-3" /> {t("waf.modeOff", "Off")}
      </Badge>
    );

  return (
    <Card data-testid="waf-policy-card">
      <CardHeader>
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-base">{title}</CardTitle>
          {badge}
        </div>
        <CardDescription className="text-xs">
          {t("waf.modeFrom", "Decided by {{from}}.", { from: FROM[effective.mode_from] ?? effective.mode_from })}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex gap-2" role="radiogroup">
          {(["detect", "block"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              className={`px-3 py-1.5 rounded-md border text-xs font-medium ${mode === m ? "border-primary bg-primary/10 text-primary" : "border-border"}`}
              data-testid={`waf-mode-${m}`}
            >
              {m === "block" ? t("waf.modeBlock", "Blocking") : t("waf.modeDetect", "Detect only")}
            </button>
          ))}
        </div>

        <div className="space-y-1.5">
          <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
            {t("waf.disabledRules", "Rules switched off")}
          </div>
          <div className="flex flex-wrap gap-1.5" data-testid="waf-disabled-rules">
            {rules.length === 0 && <span className="text-xs text-muted-foreground">{t("waf.noneDisabled", "None")}</span>}
            {rules.map((r) => (
              <Badge key={r} variant="secondary" className="gap-1 font-mono">
                {r}
                <button type="button" onClick={() => setRules(rules.filter((x) => x !== r))} aria-label={`Enable rule ${r}`} data-testid={`waf-enable-${r}`}>
                  <X className="h-3 w-3" />
                </button>
              </Badge>
            ))}
          </div>
          <div className="flex gap-2 max-w-xs">
            <Input
              value={ruleInput}
              onChange={(e) => setRuleInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addRule();
                }
              }}
              placeholder="942100"
              className="font-mono text-xs h-8"
              data-testid="waf-rule-input"
            />
            <Button type="button" size="sm" variant="outline" onClick={addRule} data-testid="waf-add-rule">
              <Plus className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>

        <div className="flex gap-2">
          <Button size="sm" disabled={busy} onClick={() => run(() => onSave(mode, rules))} data-testid="waf-save">
            {t("common.save", "Save")}
          </Button>
          {onClear && own && (
            <Button size="sm" variant="outline" disabled={busy} onClick={() => run(onClear)} data-testid="waf-clear">
              {t("waf.useInherited", "Use the inherited policy")}
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
