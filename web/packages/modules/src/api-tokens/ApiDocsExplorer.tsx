import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  FileCode,
  Terminal,
  Play,
  CheckCircle2,
  AlertCircle,
  ExternalLink,
  BookOpen,
} from "lucide-react";
import { ApiTokenItem, ApiTestCallResponse } from "./types";

interface ApiDocsExplorerProps {
  tokens: ApiTokenItem[];
  selectedTokenId?: string;
}

export function ApiDocsExplorer({ tokens, selectedTokenId }: ApiDocsExplorerProps) {
  const { t } = useTranslation();

  const [activeTokenId, setActiveTokenId] = useState<string>(
    selectedTokenId || (tokens.length > 0 ? tokens[0].id : "")
  );

  const [testEndpoint, setTestEndpoint] = useState<"/users" | "/admin/nodes" | "/billing/whmcs/suspend">("/users");
  const [testMethod, setTestMethod] = useState<"GET" | "POST">("GET");
  const [isExecuting, setIsExecuting] = useState(false);
  const [testResult, setTestResult] = useState<ApiTestCallResponse | null>(null);

  const handleEndpointSelect = (endpoint: "/users" | "/admin/nodes" | "/billing/whmcs/suspend", method: "GET" | "POST") => {
    setTestEndpoint(endpoint);
    setTestMethod(method);
    setTestResult(null);
  };

  const handleExecuteCall = async () => {
    if (!activeTokenId) return;

    setIsExecuting(true);
    setTestResult(null);
    try {
      const res = await apiFetch<ApiTestCallResponse>("/api/v1/tokens/test-call", {
        method: "POST",
        body: JSON.stringify({
          token_id: activeTokenId,
          endpoint: testEndpoint,
          method: testMethod,
        }),
      });
      setTestResult(res);
    } catch (err: any) {
      setTestResult({
        status: err?.status || 500,
        message: err?.message || "Execution failed",
      });
    } finally {
      setIsExecuting(false);
    }
  };

  const selectedToken = tokens.find((t) => t.id === activeTokenId);

  return (
    <Card className="border bg-card shadow-sm" data-testid="api-docs-explorer-card">
      <CardHeader>
        <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <BookOpen className="h-5 w-5 text-primary" />
              <CardTitle className="text-base font-semibold">
                {t("apiTokens.docsTitle", "Public OpenAPI Documentation & Scope Tester")}
              </CardTitle>
            </div>
            <CardDescription className="text-xs">
              {t(
                "apiTokens.docsDesc",
                "Interactive endpoint validation verifying token authorization and scope enforcement."
              )}
            </CardDescription>
          </div>

          <a
            href="/openapi.json"
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 text-xs text-primary hover:underline shrink-0"
            data-testid="link-raw-openapi"
          >
            <span>Raw openapi.json</span>
            <ExternalLink className="h-3.5 w-3.5" />
          </a>
        </div>
      </CardHeader>

      <CardContent className="space-y-6">
        {/* Endpoints Overview Table */}
        <div className="space-y-2">
          <span className="text-xs font-semibold text-foreground">
            Featured Integration Endpoints (WHMCS & Public API)
          </span>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
            {/* Endpoint 1 */}
            <div
              onClick={() => handleEndpointSelect("/users", "GET")}
              className={`p-3 rounded-lg border cursor-pointer transition-all ${
                testEndpoint === "/users"
                  ? "bg-primary/10 border-primary shadow-xs"
                  : "bg-muted/20 border-border hover:bg-muted/40"
              }`}
              data-testid="endpoint-btn-users"
            >
              <div className="flex items-center justify-between mb-1.5">
                <Badge variant="success" className="text-[10px] font-mono px-1.5 py-0">
                  GET
                </Badge>
                <span className="text-[10px] text-muted-foreground font-mono">Scope: users:read</span>
              </div>
              <div className="font-mono font-semibold text-foreground">/api/v1/users</div>
              <p className="text-[11px] text-muted-foreground mt-1">
                List tenant accounts, usernames, and quotas.
              </p>
            </div>

            {/* Endpoint 2 */}
            <div
              onClick={() => handleEndpointSelect("/billing/whmcs/suspend", "POST")}
              className={`p-3 rounded-lg border cursor-pointer transition-all ${
                testEndpoint === "/billing/whmcs/suspend"
                  ? "bg-primary/10 border-primary shadow-xs"
                  : "bg-muted/20 border-border hover:bg-muted/40"
              }`}
              data-testid="endpoint-btn-whmcs"
            >
              <div className="flex items-center justify-between mb-1.5">
                <Badge variant="default" className="text-[10px] font-mono px-1.5 py-0">
                  POST
                </Badge>
                <span className="text-[10px] text-muted-foreground font-mono">Scope: billing:whmcs</span>
              </div>
              <div className="font-mono font-semibold text-foreground truncate">/billing/whmcs/suspend</div>
              <p className="text-[11px] text-muted-foreground mt-1">
                Trigger automated suspension from WHMCS hook.
              </p>
            </div>

            {/* Endpoint 3 */}
            <div
              onClick={() => handleEndpointSelect("/admin/nodes", "POST")}
              className={`p-3 rounded-lg border cursor-pointer transition-all ${
                testEndpoint === "/admin/nodes"
                  ? "bg-primary/10 border-primary shadow-xs"
                  : "bg-muted/20 border-border hover:bg-muted/40"
              }`}
              data-testid="endpoint-btn-nodes"
            >
              <div className="flex items-center justify-between mb-1.5">
                <Badge variant="destructive" className="text-[10px] font-mono px-1.5 py-0">
                  POST
                </Badge>
                <span className="text-[10px] text-muted-foreground font-mono">Scope: cluster:admin</span>
              </div>
              <div className="font-mono font-semibold text-foreground">/api/v1/admin/nodes</div>
              <p className="text-[11px] text-muted-foreground mt-1">
                Privileged node enrollment (triggers 403 if unauthorized).
              </p>
            </div>
          </div>
        </div>

        {/* Live Scope Tester Console */}
        <div className="p-4 rounded-lg border bg-muted/20 space-y-4 text-xs">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
            <div className="flex items-center gap-2">
              <Terminal className="h-4 w-4 text-primary" />
              <span className="font-semibold text-foreground">Scope Authorization Tester</span>
            </div>

            <div className="flex items-center gap-2">
              <span className="text-muted-foreground text-[11px]">Select Token:</span>
              <select
                value={activeTokenId}
                onChange={(e) => {
                  setActiveTokenId(e.target.value);
                  setTestResult(null);
                }}
                className="h-8 text-xs px-2.5 rounded border border-input bg-background font-mono"
                data-testid="select-test-token"
              >
                {tokens.map((tok) => (
                  <option key={tok.id} value={tok.id}>
                    {tok.name} ({tok.token_preview})
                  </option>
                ))}
              </select>

              <Button
                size="sm"
                variant="default"
                disabled={!activeTokenId || isExecuting}
                onClick={handleExecuteCall}
                className="gap-1.5 text-xs h-8 px-3"
                data-testid="btn-run-api-test"
              >
                <Play className="h-3.5 w-3.5" />
                <span>{isExecuting ? "Executing..." : "Send Request"}</span>
              </Button>
            </div>
          </div>

          {/* Active Request Details */}
          <div className="p-2.5 rounded bg-card border font-mono text-xs flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="font-bold text-primary">{testMethod}</span>
              <span className="text-foreground">{testEndpoint}</span>
            </div>
            {selectedToken && (
              <span className="text-[11px] text-muted-foreground">
                Bearer {selectedToken.token_preview} (Scopes: {selectedToken.scopes.join(", ")})
              </span>
            )}
          </div>

          {/* Test Response Console */}
          {testResult && (
            <div
              className={`p-3 rounded-lg border font-mono text-xs space-y-2 ${
                testResult.status === 200
                  ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-800 dark:text-emerald-300"
                  : "bg-destructive/10 border-destructive/30 text-destructive"
              }`}
              data-testid="api-test-response-console"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 font-bold">
                  {testResult.status === 200 ? (
                    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                  ) : (
                    <AlertCircle className="h-4 w-4 text-destructive" />
                  )}
                  <span>HTTP {testResult.status} {testResult.status === 200 ? "OK" : "Forbidden"}</span>
                </div>
                <span className="text-[10px] text-muted-foreground">{testResult.message}</span>
              </div>

              {testResult.data && (
                <pre className="text-[11px] p-2 rounded bg-background/50 overflow-x-auto text-foreground">
                  {JSON.stringify(testResult.data, null, 2)}
                </pre>
              )}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
