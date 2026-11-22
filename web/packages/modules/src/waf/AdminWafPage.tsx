import React, { useCallback, useEffect, useState } from "react";
import { Button, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { RefreshCw, ShieldAlert } from "lucide-react";
import { PolicyEditor } from "./PolicyEditor";
import { clearAdminPolicy, getAdminPolicy, setAdminPolicy, type AdminScope, type PolicyAnswer } from "./api";

type Level = "global" | "tenant" | "domain";

interface Choice {
  id: string;
  label: string;
}

/** The firewall policy of the whole panel, of one account or of one domain. */
export function AdminWafPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [level, setLevel] = useState<Level>("global");
  const [accounts, setAccounts] = useState<Choice[]>([]);
  const [domains, setDomains] = useState<Choice[]>([]);
  const [targetId, setTargetId] = useState("");
  const [answer, setAnswer] = useState<PolicyAnswer | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const fail = useCallback(
    (err: any) => toast({ title: t("common.error"), description: err?.message || t("common.unknownError"), variant: "destructive" }),
    [t, toast],
  );

  useEffect(() => {
    apiFetch<{ items: { tenant?: { id: string; username: string } | null }[] }>("/api/v1/admin/users?limit=200")
      .then((res) => setAccounts(res.items.flatMap((u) => (u.tenant ? [{ id: u.tenant.id, label: u.tenant.username }] : []))))
      .catch(() => undefined);
    apiFetch<{ items: { id: string; unicode: string; fqdn: string }[] }>("/api/v1/admin/domains?limit=200")
      .then((res) => setDomains(res.items.map((d) => ({ id: d.id, label: d.unicode || d.fqdn }))))
      .catch(() => undefined);
  }, []);

  const scope: AdminScope | null =
    level === "global" ? { kind: "global" } : targetId ? { kind: level, id: targetId } : null;

  const load = useCallback(async () => {
    if (!scope) {
      setAnswer(null);
      return;
    }
    try {
      setIsLoading(true);
      setAnswer(await getAdminPolicy(scope));
    } catch (err) {
      fail(err);
    } finally {
      setIsLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, targetId, fail]);

  useEffect(() => {
    void load();
  }, [load]);

  const choices = level === "tenant" ? accounts : domains;

  return (
    <div className="space-y-6" data-testid="admin-waf-page">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2" data-testid="admin-waf-title">
            <ShieldAlert className="h-6 w-6 text-primary" />
            {t("waf.adminTitle", "Web Application Firewall policy")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("waf.adminSubtitle", "A domain's policy wins over its account's, which wins over the panel's.")}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <select
            value={level}
            onChange={(e) => {
              setLevel(e.target.value as Level);
              setTargetId("");
            }}
            className="h-9 rounded-md border border-input bg-background px-2.5 text-xs"
            data-testid="select-waf-level"
          >
            <option value="global">{t("waf.levelGlobal", "The whole panel")}</option>
            <option value="tenant">{t("waf.levelTenant", "One account")}</option>
            <option value="domain">{t("waf.levelDomain", "One domain")}</option>
          </select>
          {level !== "global" && (
            <select
              value={targetId}
              onChange={(e) => setTargetId(e.target.value)}
              className="h-9 rounded-md border border-input bg-background px-2.5 text-xs font-mono"
              data-testid="select-waf-target"
            >
              <option value="">—</option>
              {choices.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          )}
          <Button size="sm" variant="outline" onClick={load} disabled={isLoading} data-testid="btn-refresh-admin-waf">
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {answer && scope ? (
        <PolicyEditor
          title={level === "global" ? t("waf.levelGlobal", "The whole panel") : choices.find((c) => c.id === targetId)?.label ?? ""}
          effective={answer.effective}
          own={answer.effective.mode_from === level || !!answer.policy}
          onSave={async (mode, rules) => {
            try {
              await setAdminPolicy(scope, mode, rules);
              toast({ title: t("common.success"), description: t("waf.saved", "Firewall policy saved") });
              await load();
            } catch (err) {
              fail(err);
            }
          }}
          onClear={async () => {
            try {
              await clearAdminPolicy(scope);
              await load();
            } catch (err) {
              fail(err);
            }
          }}
        />
      ) : (
        level !== "global" && (
          <p className="text-sm text-muted-foreground">
            {t("waf.pickTarget", "Pick an account or a domain.")}
          </p>
        )
      )}
    </div>
  );
}
