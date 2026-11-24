import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import {
  ShieldCheck,
  ShieldAlert,
  Ban,
  Globe,
  Plus,
  RefreshCw,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ClientIpRule } from "./types";
import { addIpBlock, listIpBlocks, listOwnDomains, removeIpBlock, type OwnDomain } from "./api";
import { ClientIpRulesTable } from "./ClientIpRulesTable";
import { AddIpRuleDialog } from "./AddIpRuleDialog";

export function ClientFirewallPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [domains, setDomains] = useState<OwnDomain[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [rules, setRules] = useState<ClientIpRule[]>([]);
  const [limit, setLimit] = useState(100);
  const [isLoading, setIsLoading] = useState(true);
  const [isAddOpen, setIsAddOpen] = useState(false);

  const selected = domains.find((d) => d.id === selectedId);
  const selectedDomain = selected?.name ?? "";

  const failed = useCallback(
    (err: unknown) =>
      toast({
        title: t("common.error"),
        description: err instanceof Error ? err.message : t("common.unknownError"),
        variant: "destructive",
      }),
    [toast, t],
  );

  useEffect(() => {
    void (async () => {
      try {
        const list = await listOwnDomains();
        setDomains(list);
        setSelectedId((cur) => cur || list[0]?.id || "");
        if (list.length === 0) setIsLoading(false);
      } catch (err) {
        failed(err);
        setIsLoading(false);
      }
    })();
  }, [failed]);

  const fetchRules = useCallback(async () => {
    if (!selected) return;
    try {
      setIsLoading(true);
      const res = await listIpBlocks(selected);
      setRules(res.rules);
      setLimit(res.limit);
    } catch (err) {
      failed(err);
    } finally {
      setIsLoading(false);
    }
  }, [selected, failed]);

  useEffect(() => {
    void fetchRules();
  }, [fetchRules]);

  // The web server turns the address away before the site sees the request.
  const handleAddRule = async (newRule: { ip_cidr: string; notes: string }) => {
    if (!selected) return;
    try {
      await addIpBlock(selected, newRule.ip_cidr, newRule.notes);
      toast({
        title: t("common.success"),
        description: t("firewall.ipBlockedSuccess", { ip: newRule.ip_cidr, domain: selected.name }),
      });
      await fetchRules();
    } catch (err) {
      failed(err);
      throw err;
    }
  };

  const handleDeleteRule = async (ruleId: string, ipCidr: string) => {
    if (!selected) return;
    try {
      await removeIpBlock(selected, ruleId);
      toast({
        title: t("common.success"),
        description: t("firewall.ruleDeletedSuccess", { ip: ipCidr }),
      });
      await fetchRules();
    } catch (err) {
      failed(err);
    }
  };

  const blockedCount = rules.length;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Ban className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight" data-testid="firewall-page-title">
              {t("firewall.pageTitle")}
            </h1>
            <Badge variant="outline" className="border-emerald-500/30 text-emerald-600 dark:text-emerald-400">
              {t("firewall.caddyLevelEnforced")}
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("firewall.pageSubtitle")}
          </p>
        </div>

        {/* Domain Selector, Add & Refresh */}
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1.5 rounded-md border bg-background px-3 py-1.5 text-xs shadow-sm">
            <Globe className="h-3.5 w-3.5 text-muted-foreground" />
            <select
              data-testid="select-firewall-domain"
              value={selectedId}
              onChange={(e) => setSelectedId(e.target.value)}
              className="bg-transparent font-medium focus:outline-none"
            >
              {domains.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </select>
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={fetchRules}
            disabled={isLoading}
            data-testid="btn-refresh-firewall"
            className="gap-1 text-xs"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </Button>

          <Button
            size="sm"
            disabled={!selected}
            onClick={() => setIsAddOpen(true)}
            data-testid="btn-open-add-ip-rule"
            className="gap-1.5 text-xs bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("firewall.btnBlockIp")}
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <div className="rounded-lg border bg-card p-3 shadow-sm" data-testid="card-blocked-ips-count">
          <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.blockedIps")}</span>
          <p className="text-2xl font-bold font-mono text-destructive mt-0.5">{blockedCount}</p>
        </div>
        <div className="rounded-lg border bg-card p-3 shadow-sm" data-testid="card-allowed-ips-count">
          <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.limit", "Most per domain")}</span>
          <p className="text-2xl font-bold font-mono mt-0.5">{limit}</p>
        </div>
        <div className="rounded-lg border bg-card p-3 shadow-sm">
          <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.enforcementScope")}</span>
          <p className="text-xs font-semibold mt-1 truncate font-mono">{selectedDomain}</p>
        </div>
        <div className="rounded-lg border bg-card p-3 shadow-sm">
          <span className="text-[11px] text-muted-foreground uppercase">{t("firewall.layer")}</span>
          <p className="flex items-center gap-1 text-xs font-semibold text-primary mt-1">
            <ShieldCheck className="h-3.5 w-3.5 text-emerald-500" />
            <span>Caddy HTTP Handler</span>
          </p>
        </div>
      </div>

      {/* Rules Table */}
      <ClientIpRulesTable
        rules={rules}
        domain={selectedDomain}
        onDeleteRule={handleDeleteRule}
        isLoading={isLoading}
      />

      {/* Add IP Modal */}
      <AddIpRuleDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        domain={selectedDomain}
        blockOnly
        onConfirm={handleAddRule}
      />
    </div>
  );
}
