import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Globe,
  CheckCircle2,
  AlertTriangle,
  Play,
  Server,
  Zap,
  ShieldCheck,
  RefreshCw,
} from "lucide-react";
import { DnsClusterServer, DnsResolutionTestResult } from "./types";

export interface DnsResolutionTestDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  servers: DnsClusterServer[];
}

export function DnsResolutionTestDialog({
  open,
  onOpenChange,
  servers,
}: DnsResolutionTestDialogProps) {
  const { t } = useTranslation();

  const [domain, setDomain] = useState("example.com");
  const [recordType, setRecordType] = useState("A");
  const [simulatedDownServerId, setSimulatedDownServerId] = useState<string>(
    servers.find((s) => s.role === "secondary")?.id || ""
  );
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<DnsResolutionTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleRunTest = async () => {
    if (!domain.trim()) {
      setError(t("dns.cluster.test.domainRequired"));
      return;
    }

    try {
      setIsRunning(true);
      setError(null);
      setResult(null);

      const res = await apiFetch<DnsResolutionTestResult>(
        "/api/v1/admin/cluster/dns/resolve-test",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            domain: domain.trim(),
            record_type: recordType,
            simulate_offline_server_id: simulatedDownServerId || null,
          }),
        }
      );

      setResult(res);
    } catch (err: any) {
      setError(err?.message || "Failed to execute resolution test");
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-2xl p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Zap className="h-5 w-5" />
            <DialogTitle>{t("dns.cluster.test.title")}</DialogTitle>
          </div>
          <DialogDescription>{t("dns.cluster.test.desc")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 my-3">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* Test Inputs */}
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <div className="sm:col-span-2 space-y-1">
              <label htmlFor="test-domain" className="text-xs font-medium text-foreground">
                {t("dns.cluster.test.domainLabel")}
              </label>
              <Input
                id="test-domain"
                data-testid="input-test-domain"
                value={domain}
                onChange={(e) => setDomain(e.target.value)}
                placeholder="e.g. example.com"
                disabled={isRunning}
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="test-type" className="text-xs font-medium text-foreground">
                {t("dns.cluster.test.typeLabel")}
              </label>
              <select
                id="test-type"
                value={recordType}
                onChange={(e) => setRecordType(e.target.value)}
                disabled={isRunning}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
              >
                <option value="A">A</option>
                <option value="AAAA">AAAA</option>
                <option value="CNAME">CNAME</option>
                <option value="MX">MX</option>
                <option value="TXT">TXT</option>
              </select>
            </div>
          </div>

          {/* Simulated Outage Selection (DoD Requirement) */}
          <div className="p-3 rounded-lg border bg-muted/20 space-y-2">
            <div className="flex items-center justify-between">
              <label htmlFor="simulate-down-server" className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
                <span>{t("dns.cluster.test.simulateOutageLabel")}</span>
              </label>
              <Badge variant="outline" className="text-[10px] px-1.5 py-0 font-mono text-amber-600 bg-amber-500/10 border-amber-500/20">
                DoD T16.04 Check
              </Badge>
            </div>
            <p className="text-[11px] text-muted-foreground">
              {t("dns.cluster.test.simulateOutageDesc")}
            </p>
            <select
              id="simulate-down-server"
              data-testid="select-simulate-down-server"
              value={simulatedDownServerId}
              onChange={(e) => setSimulatedDownServerId(e.target.value)}
              disabled={isRunning}
              className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
            >
              <option value="">{t("dns.cluster.test.allServersOnline")}</option>
              {servers.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} ({s.hostname} - {s.role}) [Simulate Offline]
                </option>
              ))}
            </select>
          </div>

          {/* Live Result View */}
          {result && (
            <div
              data-testid="dns-test-result-box"
              className={`p-4 rounded-lg border space-y-3 transition-all ${
                result.resolved
                  ? "bg-emerald-500/10 border-emerald-500/30 text-foreground"
                  : "bg-destructive/10 border-destructive/30 text-destructive"
              }`}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {result.resolved ? (
                    <CheckCircle2 className="h-5 w-5 text-emerald-600 shrink-0" />
                  ) : (
                    <AlertTriangle className="h-5 w-5 text-destructive shrink-0" />
                  )}
                  <span className="font-semibold text-sm">
                    {result.resolved
                      ? t("dns.cluster.test.resolvedSuccess")
                      : t("dns.cluster.test.resolvedFailed")}
                  </span>
                </div>
                <Badge
                  variant="outline"
                  data-testid="failover-status-badge"
                  className={`text-[10px] font-mono px-2 py-0.5 ${
                    result.redundancy_active
                      ? "bg-emerald-500/20 text-emerald-700 border-emerald-500/40 font-semibold"
                      : "bg-muted text-muted-foreground"
                  }`}
                >
                  {result.redundancy_active
                    ? t("dns.cluster.test.redundancyActive")
                    : t("dns.cluster.test.singleServerActive")}
                </Badge>
              </div>

              <p className="text-xs text-muted-foreground">{result.message}</p>

              <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 text-xs pt-1 border-t border-emerald-500/20">
                <div>
                  <span className="text-[11px] text-muted-foreground block">
                    {t("dns.cluster.test.answeringServer")}:
                  </span>
                  <span className="font-mono font-medium" data-testid="answering-server-name">
                    {result.answering_server}
                  </span>
                </div>
                <div>
                  <span className="text-[11px] text-muted-foreground block">
                    {t("dns.cluster.test.answeringIp")}:
                  </span>
                  <span className="font-mono">{result.answering_ip}</span>
                </div>
                <div>
                  <span className="text-[11px] text-muted-foreground block">
                    {t("dns.cluster.test.responseTime")}:
                  </span>
                  <span className="font-mono">{result.resolution_time_ms} ms</span>
                </div>
              </div>

              {result.records.length > 0 && (
                <div className="pt-2">
                  <span className="text-[11px] text-muted-foreground block mb-1">
                    {t("dns.cluster.test.answers")}:
                  </span>
                  <div className="flex flex-wrap gap-1">
                    {result.records.map((r, i) => (
                      <Badge
                        key={i}
                        variant="secondary"
                        className="font-mono text-xs px-2 py-0.5"
                      >
                        {r}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="pt-3 border-t gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={isRunning}
            onClick={() => onOpenChange(false)}
          >
            {t("common.close")}
          </Button>
          <Button
            type="button"
            data-testid="btn-run-dns-resolution-test"
            disabled={isRunning}
            onClick={handleRunTest}
            className="gap-2"
          >
            {isRunning ? (
              <RefreshCw className="h-4 w-4 animate-spin" />
            ) : (
              <Play className="h-4 w-4" />
            )}
            {isRunning ? t("common.processing") : t("dns.cluster.test.runTestBtn")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
