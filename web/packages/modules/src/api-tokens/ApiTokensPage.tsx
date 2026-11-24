import React, { useState, useEffect, useCallback } from "react";
import { Button, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { Plus, Key, RefreshCw } from "lucide-react";
import { ApiTokenItem, CreateTokenResponse } from "./types";
import { ApiTokensTable } from "./ApiTokensTable";
import { CreateTokenDialog } from "./CreateTokenDialog";
import { ApiDocsExplorer } from "./ApiDocsExplorer";

const INITIAL_TOKENS: ApiTokenItem[] = [
  {
    id: "tok-whmcs-01",
    name: "WHMCS Primary Production Node",
    token_preview: "dhp_live_9f82...104a",
    scopes: ["users:read", "users:write", "billing:whmcs"],
    created_at: new Date(Date.now() - 86400000 * 14).toISOString(),
    expires_at: null,
    last_used_at: new Date(Date.now() - 3600000).toISOString(),
  },
  {
    id: "tok-metrics-02",
    name: "Prometheus Exporter Scraper",
    token_preview: "dhp_live_4a11...99bc",
    scopes: ["users:read", "domains:read"],
    created_at: new Date(Date.now() - 86400000 * 30).toISOString(),
    expires_at: new Date(Date.now() + 86400000 * 60).toISOString(),
    last_used_at: new Date(Date.now() - 60000).toISOString(),
  },
];

export function ApiTokensPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [tokens, setTokens] = useState<ApiTokenItem[]>(INITIAL_TOKENS);
  const [createOpen, setCreateOpen] = useState(false);
  const [selectedTokenForTest, setSelectedTokenForTest] = useState<string | undefined>(
    INITIAL_TOKENS[0]?.id
  );
  const [isLoading, setIsLoading] = useState(false);

  const fetchTokens = useCallback(async () => {
    setIsLoading(true);
    try {
      const res = await apiFetch<{ tokens: ApiTokenItem[] }>("/api/v1/tokens");
      if (res?.tokens) {
        setTokens(res.tokens);
        if (res.tokens.length > 0 && !selectedTokenForTest) {
          setSelectedTokenForTest(res.tokens[0].id);
        }
      }
    } catch {
      // Keep initial
    } finally {
      setIsLoading(false);
    }
  }, [selectedTokenForTest]);

  useEffect(() => {
    fetchTokens();
  }, [fetchTokens]);

  const handleCreated = (res: CreateTokenResponse) => {
    setTokens((prev) => [res.token, ...prev]);
    setSelectedTokenForTest(res.token.id);
  };

  const handleRevoke = async (id: string) => {
    try {
      await apiFetch(`/api/v1/tokens/${id}`, { method: "DELETE" });
      setTokens((prev) => prev.filter((t) => t.id !== id));
      toast({
        title: t("apiTokens.revokedTitle", "Token Revoked"),
        description: t("apiTokens.revokedDesc", "API token was permanently deleted and revoked."),
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || "Failed to revoke token",
        variant: "destructive",
      });
    }
  };

  return (
    <div className="space-y-6" data-testid="api-tokens-page">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">
            {t("apiTokens.pageTitle", "API Tokens & Public Developer API")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t(
              "apiTokens.pageSubtitle",
              "Generate scoped Bearer tokens for billing integration (WHMCS), CLI clients, and public REST endpoints."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchTokens}
            disabled={isLoading}
            className="gap-2 text-xs h-9"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh", "Refresh")}</span>
          </Button>

          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            className="gap-2 text-xs h-9 font-semibold"
            data-testid="btn-open-create-token"
          >
            <Plus className="h-4 w-4" />
            <span>{t("apiTokens.generateButton", "Generate New Token")}</span>
          </Button>
        </div>
      </div>

      {/* Active Tokens Table */}
      <ApiTokensTable
        tokens={tokens}
        onRevoke={handleRevoke}
        onSelectTokenForTest={(tok) => setSelectedTokenForTest(tok.id)}
      />

      {/* Interactive Scope & Public Docs Explorer */}
      <ApiDocsExplorer tokens={tokens} selectedTokenId={selectedTokenForTest} />

      {/* Create Dialog */}
      <CreateTokenDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={handleCreated}
      />
    </div>
  );
}
