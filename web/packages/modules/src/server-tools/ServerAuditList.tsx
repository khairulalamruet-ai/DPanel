import React from "react";
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
  Badge,
} from "@builtpanel/ui";
import { ShieldCheck, History, ExternalLink } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ServerAuditEvent } from "./types";

interface ServerAuditListProps {
  events: ServerAuditEvent[];
  isLoading?: boolean;
}

export function ServerAuditList({ events, isLoading }: ServerAuditListProps) {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-2 text-base font-semibold">
            <History className="h-4 w-4 text-primary" />
            <span data-testid="title-server-audit-trail">{t("serverTools.auditTrailTitle")}</span>
          </CardTitle>
          <Badge variant="outline" className="gap-1 text-xs text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
            <ShieldCheck className="h-3.5 w-3.5" />
            <span>{t("serverTools.dodAuditEnforced")}</span>
          </Badge>
        </div>
        <CardDescription className="text-xs text-muted-foreground">
          {t("serverTools.auditTrailDescription")}
        </CardDescription>
      </CardHeader>

      <CardContent>
        {isLoading ? (
          <p className="py-4 text-center text-xs text-muted-foreground">{t("common.loading")}</p>
        ) : events.length === 0 ? (
          <p className="py-4 text-center text-xs text-muted-foreground">{t("common.noData")}</p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="text-xs">{t("serverTools.auditAction")}</TableHead>
                  <TableHead className="text-xs">{t("serverTools.auditActor")}</TableHead>
                  <TableHead className="text-xs">{t("serverTools.auditDetails")}</TableHead>
                  <TableHead className="text-xs">{t("serverTools.auditTimestamp")}</TableHead>
                  <TableHead className="text-xs text-right">{t("common.status")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {events.map((evt) => (
                  <TableRow key={evt.id} data-testid={`audit-row-${evt.id}`}>
                    <TableCell className="font-mono text-xs font-medium">
                      {evt.action}
                    </TableCell>
                    <TableCell className="text-xs">{evt.actor}</TableCell>
                    <TableCell className="text-xs text-muted-foreground">{evt.details}</TableCell>
                    <TableCell className="text-xs font-mono text-muted-foreground">
                      {evt.created_at}
                    </TableCell>
                    <TableCell className="text-right">
                      <Badge
                        variant={evt.status === "success" ? "default" : "destructive"}
                        className="text-[10px]"
                      >
                        {evt.status}
                      </Badge>
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
