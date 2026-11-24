import { WafEventsCard } from "./WafEventsCard";
import React, { useCallback, useEffect, useState } from "react";
import { Button, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { RefreshCw, ShieldCheck } from "lucide-react";
import { PolicyEditor } from "./PolicyEditor";
import { clearClientDomainPolicy, listClientDomainPolicies, setClientDomainPolicy, type DomainPolicy } from "./api";

/** The web application firewall of each of the account's domains: its mode and the rules switched off. */
export function ClientWafPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [domains, setDomains] = useState<DomainPolicy[]>([]);
  const [selected, setSelected] = useState("");
  const [isLoading, setIsLoading] = useState(true);

  const fail = useCallback(
    (err: any) =>
      toast({ title: t("common.error"), description: err?.message || t("common.unknownError"), variant: "destructive" }),
    [t, toast],
  );

  const load = useCallback(async () => {
    try {
      setIsLoading(true);
      const list = await listClientDomainPolicies();
      setDomains(list);
      setSelected((current) => (list.some((d) => d.domain_id === current) ? current : (list[0]?.domain_id ?? "")));
    } catch (err) {
      fail(err);
    } finally {
      setIsLoading(false);
    }
  }, [fail]);

  useEffect(() => {
    void load();
  }, [load]);

  const current = domains.find((d) => d.domain_id === selected);

  return (
    <div className="space-y-6" data-testid="client-waf-page">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2" data-testid="waf-page-title">
            <ShieldCheck className="h-6 w-6 text-primary" />
            {t("waf.title", "Web Application Firewall")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("waf.subtitle", "Detect or block attacks on each of your domains, and switch off rules that stop something that is fine.")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <select
            value={selected}
            onChange={(e) => setSelected(e.target.value)}
            className="h-9 rounded-md border border-input bg-background px-2.5 text-xs font-mono"
            data-testid="select-waf-domain"
          >
            {domains.map((d) => (
              <option key={d.domain_id} value={d.domain_id}>
                {d.fqdn}
              </option>
            ))}
          </select>
          <Button size="sm" variant="outline" onClick={load} disabled={isLoading} data-testid="btn-refresh-waf">
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {current ? (
        <PolicyEditor
          title={current.fqdn}
          effective={current.effective}
          own={current.own}
          onSave={async (mode, rules) => {
            try {
              await setClientDomainPolicy(current.domain_id, mode, rules);
              toast({ title: t("common.success"), description: t("waf.saved", "Firewall policy saved") });
              await load();
            } catch (err) {
              fail(err);
            }
          }}
          onClear={async () => {
            try {
              await clearClientDomainPolicy(current.domain_id);
              await load();
            } catch (err) {
              fail(err);
            }
          }}
        />
      ) : (
        !isLoading && <p className="text-sm text-muted-foreground">{t("waf.noDomains", "There are no domains yet.")}</p>
      )}
      <WafEventsCard />
    </div>
  );
}
