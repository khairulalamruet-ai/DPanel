import React from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { History, CheckCircle2, RotateCcw, AlertTriangle } from "lucide-react";
import { UpdateHistoryItem } from "./types";

interface UpdateHistoryTableProps {
  history: UpdateHistoryItem[];
}

export function UpdateHistoryTable({ history }: UpdateHistoryTableProps) {
  const { t } = useTranslation();

  return (
    <Card className="border bg-card shadow-sm" data-testid="update-history-card">
      <CardHeader>
        <div className="flex items-center gap-2">
          <History className="h-5 w-5 text-primary" />
          <CardTitle className="text-base font-semibold">
            {t("update.history.title", "Release History & Rollback Ledger")}
          </CardTitle>
        </div>
        <CardDescription className="text-xs">
          {t(
            "update.history.desc",
            "Audit trail of all package upgrades, schema migrations, and emergency rollback events."
          )}
        </CardDescription>
      </CardHeader>

      <CardContent>
        {history.length === 0 ? (
          <div className="text-center py-6 text-xs text-muted-foreground">
            {t("update.history.empty", "No update history recorded yet.")}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs" data-testid="update-history-table">
              <thead>
                <tr className="border-b bg-muted/40 text-muted-foreground">
                  <th className="py-2.5 px-3 font-medium">Timestamp</th>
                  <th className="py-2.5 px-3 font-medium">Version Transition</th>
                  <th className="py-2.5 px-3 font-medium">Channel</th>
                  <th className="py-2.5 px-3 font-medium">Status</th>
                  <th className="py-2.5 px-3 font-medium">Details</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {history.map((item) => (
                  <tr key={item.id} className="hover:bg-muted/20 transition-colors">
                    <td className="py-2 px-3 text-muted-foreground whitespace-nowrap font-mono text-[11px]">
                      {new Date(item.timestamp).toLocaleString()}
                    </td>
                    <td className="py-2 px-3 font-mono font-semibold text-foreground">
                      v{item.from_version} → v{item.to_version}
                    </td>
                    <td className="py-2 px-3 font-mono capitalize">
                      {item.channel}
                    </td>
                    <td className="py-2 px-3">
                      {item.status === "success" && (
                        <Badge variant="success" className="gap-1 text-[10px]">
                          <CheckCircle2 className="h-3 w-3" />
                          <span>Applied</span>
                        </Badge>
                      )}
                      {item.status === "rolled_back" && (
                        <Badge variant="destructive" className="gap-1 text-[10px]">
                          <RotateCcw className="h-3 w-3" />
                          <span>Rolled Back</span>
                        </Badge>
                      )}
                      {item.status === "failed" && (
                        <Badge variant="destructive" className="gap-1 text-[10px]">
                          <AlertTriangle className="h-3 w-3" />
                          <span>Failed</span>
                        </Badge>
                      )}
                    </td>
                    <td className="py-2 px-3 text-muted-foreground max-w-[280px] truncate" title={item.details}>
                      {item.details}
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
