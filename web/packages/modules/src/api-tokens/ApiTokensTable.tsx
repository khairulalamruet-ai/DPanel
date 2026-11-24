import React from "react";
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
import { Key, Trash2, ShieldCheck, Clock } from "lucide-react";
import { ApiTokenItem } from "./types";

interface ApiTokensTableProps {
  tokens: ApiTokenItem[];
  onRevoke: (id: string) => void;
  onSelectTokenForTest?: (token: ApiTokenItem) => void;
}

export function ApiTokensTable({
  tokens,
  onRevoke,
  onSelectTokenForTest,
}: ApiTokensTableProps) {
  const { t } = useTranslation();

  return (
    <Card className="border bg-card shadow-sm" data-testid="api-tokens-table-card">
      <CardHeader>
        <div className="flex items-center gap-2">
          <Key className="h-5 w-5 text-primary" />
          <CardTitle className="text-base font-semibold">
            {t("apiTokens.tableTitle", "Active API Access Tokens")}
          </CardTitle>
        </div>
        <CardDescription className="text-xs">
          {t(
            "apiTokens.tableDesc",
            "Manage Bearer credentials authorized to invoke Built Panel REST endpoints."
          )}
        </CardDescription>
      </CardHeader>

      <CardContent>
        {tokens.length === 0 ? (
          <div className="text-center py-8 text-xs text-muted-foreground">
            {t("apiTokens.noTokens", "No API tokens created yet. Generate one above to get started.")}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs" data-testid="tokens-table">
              <thead>
                <tr className="border-b bg-muted/40 text-muted-foreground">
                  <th className="py-2.5 px-3 font-medium">Name & Token</th>
                  <th className="py-2.5 px-3 font-medium">Granted Scopes</th>
                  <th className="py-2.5 px-3 font-medium">Expires</th>
                  <th className="py-2.5 px-3 font-medium">Last Used</th>
                  <th className="py-2.5 px-3 font-medium text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {tokens.map((token) => (
                  <tr
                    key={token.id}
                    className="hover:bg-muted/20 transition-colors"
                    data-testid={`token-row-${token.id}`}
                  >
                    <td className="py-3 px-3">
                      <div className="font-semibold text-foreground">{token.name}</div>
                      <div className="font-mono text-[11px] text-muted-foreground mt-0.5">
                        {token.token_preview}
                      </div>
                    </td>

                    <td className="py-3 px-3">
                      <div className="flex flex-wrap gap-1 max-w-[320px]">
                        {token.scopes.map((scope) => (
                          <Badge
                            key={scope}
                            variant="secondary"
                            className="text-[10px] font-mono px-1.5 py-0"
                          >
                            {scope}
                          </Badge>
                        ))}
                      </div>
                    </td>

                    <td className="py-3 px-3 text-muted-foreground font-mono text-[11px] whitespace-nowrap">
                      {token.expires_at ? (
                        new Date(token.expires_at).toLocaleDateString()
                      ) : (
                        <span className="text-emerald-600 dark:text-emerald-400">Never</span>
                      )}
                    </td>

                    <td className="py-3 px-3 text-muted-foreground font-mono text-[11px] whitespace-nowrap">
                      {token.last_used_at ? (
                        new Date(token.last_used_at).toLocaleDateString()
                      ) : (
                        <span>Never used</span>
                      )}
                    </td>

                    <td className="py-3 px-3 text-right">
                      <div className="flex items-center justify-end gap-2">
                        {onSelectTokenForTest && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => onSelectTokenForTest(token)}
                            className="h-7 text-[11px] px-2 text-primary hover:bg-primary/10"
                            data-testid={`btn-test-token-${token.id}`}
                          >
                            Test Scope
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => onRevoke(token.id)}
                          className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10"
                          title="Revoke Token"
                          data-testid={`btn-revoke-token-${token.id}`}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
