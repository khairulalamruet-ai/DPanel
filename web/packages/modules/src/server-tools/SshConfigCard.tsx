import React from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
} from "@builtpanel/ui";
import { ShieldCheck, ShieldAlert, KeyRound, Terminal, CheckCircle2, XCircle } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { SshConfigInfo } from "./types";

interface SshConfigCardProps {
  sshConfig: SshConfigInfo | null;
  isLoading?: boolean;
}

export function SshConfigCard({ sshConfig, isLoading }: SshConfigCardProps) {
  const { t } = useTranslation();

  if (isLoading || !sshConfig) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base font-semibold">{t("serverTools.sshConfigTitle")}</CardTitle>
          <CardDescription className="text-xs text-muted-foreground">{t("common.loading")}</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  const isHardened = sshConfig.security_rating === "hardened";

  return (
    <Card className="flex flex-col justify-between">
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-2 text-base font-semibold">
            <Terminal className="h-4 w-4 text-primary" />
            <span data-testid="title-ssh-config">{t("serverTools.sshConfigTitle")}</span>
          </CardTitle>
          <Badge
            variant={isHardened ? "default" : "outline"}
            className="gap-1 font-mono text-xs"
            data-testid="badge-ssh-rating"
          >
            {isHardened ? <ShieldCheck className="h-3 w-3" /> : <ShieldAlert className="h-3 w-3" />}
            {t(`serverTools.rating.${sshConfig.security_rating}`)}
          </Badge>
        </div>
        <CardDescription className="text-xs text-muted-foreground">
          {t("serverTools.sshConfigDescription")}
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-3">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <div className="flex items-center justify-between rounded border p-2 bg-muted/10">
            <span className="text-muted-foreground">{t("serverTools.sshPort")}:</span>
            <span className="font-mono font-medium" data-testid="text-ssh-port">{sshConfig.port}</span>
          </div>
          <div className="flex items-center justify-between rounded border p-2 bg-muted/10">
            <span className="text-muted-foreground">{t("serverTools.maxAuthTries")}:</span>
            <span className="font-mono font-medium">{sshConfig.max_auth_tries}</span>
          </div>
          <div className="flex items-center justify-between rounded border p-2 bg-muted/10">
            <span className="text-muted-foreground">{t("serverTools.permitRootLogin")}:</span>
            <span className="flex items-center gap-1 font-mono font-medium" data-testid="text-ssh-root-login">
              {sshConfig.permit_root_login === "no" ? (
                <>
                  <CheckCircle2 className="h-3 w-3 text-emerald-500" />
                  <span>no</span>
                </>
              ) : (
                <>
                  <XCircle className="h-3 w-3 text-amber-500" />
                  <span>{sshConfig.permit_root_login}</span>
                </>
              )}
            </span>
          </div>
          <div className="flex items-center justify-between rounded border p-2 bg-muted/10">
            <span className="text-muted-foreground">{t("serverTools.passwordAuth")}:</span>
            <span className="flex items-center gap-1 font-mono font-medium" data-testid="text-ssh-password-auth">
              {!sshConfig.password_authentication ? (
                <>
                  <CheckCircle2 className="h-3 w-3 text-emerald-500" />
                  <span>disabled</span>
                </>
              ) : (
                <>
                  <XCircle className="h-3 w-3 text-amber-500" />
                  <span>enabled</span>
                </>
              )}
            </span>
          </div>
        </div>

        <div className="rounded border bg-muted/20 p-2.5 text-xs">
          <div className="flex items-center justify-between mb-1.5">
            <span className="flex items-center gap-1.5 font-medium text-foreground">
              <KeyRound className="h-3.5 w-3.5 text-primary" />
              <span>{t("serverTools.authorizedKeys")}</span>
            </span>
            <Badge variant="outline" className="font-mono text-[11px]" data-testid="badge-ssh-keys-count">
              {sshConfig.authorized_keys_count} keys
            </Badge>
          </div>
          <p className="text-[11px] text-muted-foreground">
            {t("serverTools.pubkeyAuthActive")}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
