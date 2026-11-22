import React, { useState, useMemo } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  Button,
  Badge,
  Input,
} from "@builtpanel/ui";
import { ShieldCheck, Ban, CheckCircle2, Trash2, Search, Filter } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ClientIpRule } from "./types";

interface ClientIpRulesTableProps {
  rules: ClientIpRule[];
  domain: string;
  onDeleteRule: (ruleId: string, ipCidr: string) => Promise<void>;
  isLoading?: boolean;
}

export function ClientIpRulesTable({
  rules,
  domain,
  onDeleteRule,
  isLoading,
}: ClientIpRulesTableProps) {
  const { t } = useTranslation();
  const [searchQuery, setSearchQuery] = useState("");
  const [actionFilter, setActionFilter] = useState<string>("all");

  const filteredRules = useMemo(() => {
    return rules.filter((r) => {
      if (actionFilter !== "all" && r.action !== actionFilter) return false;
      if (!searchQuery.trim()) return true;

      const q = searchQuery.toLowerCase();
      return (
        r.ip_cidr.toLowerCase().includes(q) ||
        (r.notes && r.notes.toLowerCase().includes(q))
      );
    });
  }, [rules, searchQuery, actionFilter]);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-base font-semibold">
              <Ban className="h-4 w-4 text-destructive" />
              <span data-testid="title-ip-rules">{t("firewall.domainIpRulesTitle")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground mt-0.5">
              {t("firewall.domainIpRulesDescription", { domain })}
            </CardDescription>
          </div>

          <div className="flex items-center gap-2">
            <Badge variant="outline" className="font-mono text-xs" data-testid="badge-ip-rules-count">
              {filteredRules.length} {t("firewall.rulesActive")}
            </Badge>
          </div>
        </div>

        {/* Filter Bar */}
        <div className="flex flex-wrap items-center gap-2 pt-2">
          <div className="relative flex-1 min-w-[200px]">
            <Search className="absolute left-2.5 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              data-testid="input-ip-search"
              placeholder={t("firewall.searchIpPlaceholder")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-8 text-xs h-8"
            />
          </div>

          <select
            data-testid="select-rule-action-filter"
            value={actionFilter}
            onChange={(e) => setActionFilter(e.target.value)}
            className="rounded-md border border-input bg-background px-2.5 py-1 text-xs h-8 shadow-sm focus:outline-none"
          >
            <option value="all">{t("firewall.allRuleTypes")}</option>
            <option value="block">{t("firewall.onlyBlocked")}</option>
          </select>
        </div>
      </CardHeader>

      <CardContent>
        {isLoading ? (
          <p className="py-8 text-center text-xs text-muted-foreground">{t("common.loading")}</p>
        ) : filteredRules.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-center" data-testid="empty-ip-rules">
            <ShieldCheck className="h-8 w-8 text-emerald-500 mb-2 opacity-80" />
            <p className="text-sm font-medium text-foreground">{t("firewall.noRulesTitle")}</p>
            <p className="text-xs text-muted-foreground mt-1 max-w-sm">
              {t("firewall.noRulesDescription")}
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="text-xs">{t("firewall.ipOrCidr")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.action")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.ruleNotes")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.createdAt")}</TableHead>
                  <TableHead className="text-xs text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredRules.map((rule) => (
                  <TableRow key={rule.id} data-testid={`ip-rule-row-${rule.id}`}>
                    <TableCell className="font-mono text-xs font-semibold">
                      {rule.ip_cidr}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={rule.action === "block" ? "destructive" : "default"}
                        className={`text-[10px] font-mono uppercase ${rule.action === "allow" ? "bg-emerald-600 hover:bg-emerald-600" : ""}`}
                        data-testid={`badge-rule-action-${rule.id}`}
                      >
                        {rule.action}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground max-w-xs truncate">
                      {rule.notes || "—"}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {rule.created_at}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => onDeleteRule(rule.id, rule.ip_cidr)}
                        data-testid={`btn-delete-rule-${rule.id}`}
                        className="h-7 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
                        title={rule.action === "block" ? t("firewall.unblockButton") : t("common.delete")}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                        <span className="ml-1 text-[11px]">{rule.action === "block" ? t("firewall.unblock") : t("common.delete")}</span>
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
