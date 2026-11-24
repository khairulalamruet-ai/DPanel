import React from "react";
import { Card, CardHeader, CardTitle, CardContent, Button, useToast } from "@builtpanel/ui";
import { Server, Copy, Check, Shield } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { FtpConnectionInfo } from "./types";

interface FtpConnectionInfoCardProps {
  info: FtpConnectionInfo | null;
}

export function FtpConnectionInfoCard({ info }: FtpConnectionInfoCardProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [copiedKey, setCopiedKey] = React.useState<string | null>(null);

  const copyToClipboard = (key: string, text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedKey(key);
    toast({
      title: t("common.copied"),
      description: text,
    });
    setTimeout(() => setCopiedKey(null), 2000);
  };

  if (!info) return null;

  return (
    <Card className="bg-muted/30 border-primary/20" data-testid="ftp-connection-card">
      <CardHeader className="py-3 px-5 border-b">
        <div className="flex items-center justify-between">
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Server className="w-4 h-4 text-primary" />
            <span>{t("ftp.connectionSettings")}</span>
          </CardTitle>
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Shield className="w-3.5 h-3.5 text-emerald-500" />
            <span>{info.encryption}</span>
          </div>
        </div>
      </CardHeader>

      <CardContent className="p-4 grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-4">
        {/* Host */}
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground font-medium">{t("ftp.host")}</span>
          <div className="flex items-center justify-between p-2 rounded bg-background border text-xs font-mono">
            <span className="truncate" data-testid="ftp-info-host">{info.host}</span>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 w-6 p-0 shrink-0"
              onClick={() => copyToClipboard("host", info.host)}
              title={t("common.copy")}
              data-testid="btn-copy-host"
            >
              {copiedKey === "host" ? <Check className="w-3 h-3 text-emerald-500" /> : <Copy className="w-3 h-3" />}
            </Button>
          </div>
        </div>

        {/* Port */}
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground font-medium">{t("ftp.port")}</span>
          <div className="flex items-center justify-between p-2 rounded bg-background border text-xs font-mono">
            <span data-testid="ftp-info-port">{info.port}</span>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 w-6 p-0 shrink-0"
              onClick={() => copyToClipboard("port", String(info.port))}
              title={t("common.copy")}
              data-testid="btn-copy-port"
            >
              {copiedKey === "port" ? <Check className="w-3 h-3 text-emerald-500" /> : <Copy className="w-3 h-3" />}
            </Button>
          </div>
        </div>

        {/* Protocol */}
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground font-medium">{t("ftp.protocol")}</span>
          <div className="flex items-center justify-between p-2 rounded bg-background border text-xs font-mono">
            <span className="truncate" data-testid="ftp-info-protocol">{info.protocol}</span>
          </div>
        </div>

        {/* Encryption */}
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground font-medium">{t("ftp.encryption")}</span>
          <div className="flex items-center justify-between p-2 rounded bg-background border text-xs font-mono">
            <span className="truncate" data-testid="ftp-info-encryption">{info.encryption}</span>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
