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
import { Network, ShieldCheck, Lock, Trash2, Plus } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { NodePortRule } from "./types";
import { AddPortDialog } from "./AddPortDialog";

interface AdminPortsTableProps {
  ports: NodePortRule[];
  onOpenPort: (port: number, protocol: "tcp" | "udp", serviceName: string) => Promise<void>;
  onClosePort: (portId: string, port: number) => Promise<void>;
  isLoading?: boolean;
}

export function AdminPortsTable({
  ports,
  onOpenPort,
  onClosePort,
  isLoading,
}: AdminPortsTableProps) {
  const { t } = useTranslation();
  const [isAddOpen, setIsAddOpen] = useState(false);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-base font-semibold">
              <Network className="h-4 w-4 text-primary" />
              <span data-testid="title-inbound-ports">{t("firewall.inboundPortsTitle")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground mt-0.5">
              {t("firewall.inboundPortsDescription")}
            </CardDescription>
          </div>

          <Button
            size="sm"
            onClick={() => setIsAddOpen(true)}
            data-testid="btn-open-add-port"
            className="gap-1.5 text-xs"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("firewall.btnOpenPort")}
          </Button>
        </div>
      </CardHeader>

      <CardContent>
        {isLoading ? (
          <p className="py-8 text-center text-xs text-muted-foreground">{t("common.loading")}</p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="text-xs">{t("firewall.portProtocol")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.serviceDescription")}</TableHead>
                  <TableHead className="text-xs">{t("firewall.protectionStatus")}</TableHead>
                  <TableHead className="text-xs text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {ports.map((p) => (
                  <TableRow key={p.id} data-testid={`port-row-${p.port}`}>
                    <TableCell className="font-mono text-xs font-semibold">
                      <span className="text-primary">{p.port}</span>
                      <span className="text-muted-foreground uppercase text-[10px] ml-1">/{p.protocol}</span>
                    </TableCell>
                    <TableCell className="text-xs font-medium">{p.service_name}</TableCell>
                    <TableCell>
                      {p.is_system_protected ? (
                        <Badge variant="outline" className="gap-1 font-mono text-[10px] border-emerald-500/30 text-emerald-600 dark:text-emerald-400">
                          <Lock className="h-3 w-3" />
                          <span>{t("firewall.protectedNoLockout")}</span>
                        </Badge>
                      ) : (
                        <Badge variant="secondary" className="font-mono text-[10px]">
                          {t("firewall.customRule")}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      {p.is_system_protected ? (
                        <span className="text-[11px] text-muted-foreground italic">
                          {t("firewall.permanentRule")}
                        </span>
                      ) : (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => onClosePort(p.id, p.port)}
                          data-testid={`btn-close-port-${p.port}`}
                          className="h-7 text-xs text-destructive hover:bg-destructive/10"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                          <span className="ml-1 text-[11px]">{t("firewall.closePort")}</span>
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        <AddPortDialog
          open={isAddOpen}
          onOpenChange={setIsAddOpen}
          onConfirm={onOpenPort}
        />
      </CardContent>
    </Card>
  );
}
