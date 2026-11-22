import React, { useState } from "react";
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
} from "@builtpanel/ui";
import { Ban, ShieldAlert, Zap, Plus, RotateCcw } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { GlobalIpBan } from "./types";
import { AddGlobalBanDialog } from "./AddGlobalBanDialog";

interface AdminBansTableProps {
  bans: GlobalIpBan[];
  onAddBan: (ip: string, reason: string, durationHours: number) => Promise<void>;
  onUnban: (banId: string, ip: string) => Promise<void>;
  isLoading?: boolean;
}

export function AdminBansTable({
  bans,
  onAddBan,
  onUnban,
  isLoading,
}: AdminBansTableProps) {
  const { t } = useTranslation();
  const [isAddOpen, setIsAddOpen] = useState(false);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-base font-semibold">
              <Ban className="h-4 w-4 text-destructive" />
              <span data-testid="title-global-bans">{t("firewall.globalBansTitle")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground mt-0.5">
              {t("firewall.globalBansDescription")}
            </CardDescription>
          </div>

          <div className="flex items-center gap-2">
            <Badge variant="outline" className="border-amber-500/30 text-amber-600 dark:text-amber-400 text-xs gap-1">
              <Zap className="h-3 w-3" />
              <span>{t("firewall.bruteforceAutoActive")}</span>
            </Badge>

            <Button
              size="sm"
              variant="destructive"
              onClick={() => setIsAddOpen(true)}
              data-testid="btn-open-add-ban"
              className="gap-1.5 text-xs"
            >
              <Plus className="h-3.5 w-3.5" />
              {t("firewall.addIpBan")}
            </Button>
          </div>
        </div>
      </CardHeader>

      <CardContent>
        {isLoading ? (
          <p className="py-8 text-center text-xs text-muted-foreground">{t("common.loading")}</p>
        ) : bans.length === 0 ? (
          <p className="py-8 text-center text-xs text-muted-foreground">{t("firewall.noActiveBans")}</p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="text-xs">{t("firewall.bannedIp")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.banSource")}</TableHead>
                                    <TableHead className="text-xs text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {bans.map((ban) => (
                  <TableRow key={ban.id} data-testid={`ban-row-${ban.id}`}>
                    <TableCell className="font-mono text-xs font-semibold text-destructive">
                      {ban.ip}
                    </TableCell>
                    <TableCell>
                      {ban.source === "bruteforce_auto" ? (
                        <Badge variant="outline" className="gap-1 border-amber-500/30 text-amber-600 dark:text-amber-400 font-mono text-[10px]">
                          <Zap className="h-3 w-3" />
                          <span>Fail2Ban ({ban.service})</span>
                        </Badge>
                      ) : (
                        <Badge variant="secondary" className="font-mono text-[10px]">
                          Admin Manual
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => onUnban(ban.id, ban.ip)}
                        data-testid={`btn-unban-${ban.id}`}
                        className="h-7 text-xs gap-1 hover:bg-emerald-50 dark:hover:bg-emerald-950/30 hover:text-emerald-600"
                      >
                        <RotateCcw className="h-3 w-3" />
                        <span>{t("firewall.unbanButton")}</span>
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        <AddGlobalBanDialog
          open={isAddOpen}
          onOpenChange={setIsAddOpen}
          onConfirm={onAddBan}
        />
      </CardContent>
    </Card>
  );
}
