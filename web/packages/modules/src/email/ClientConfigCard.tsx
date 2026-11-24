import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Mail,
  ShieldCheck,
  Server,
  Copy,
  Check,
  Laptop,
  Smartphone,
} from "lucide-react";
import { EmailClientConfig } from "./types";

interface ClientConfigCardProps {
  domain: string;
  config: EmailClientConfig | null;
  loading?: boolean;
}

export function ClientConfigCard({ domain, config, loading }: ClientConfigCardProps) {
  const { t } = useTranslation();
  const [copiedField, setCopiedField] = useState<string | null>(null);

  const effectiveConfig: EmailClientConfig = config || {
    incoming_server: `mail.${domain}`,
    incoming_imap_port: 993,
    incoming_pop3_port: 995,
    incoming_security: "SSL/TLS",
    outgoing_server: `mail.${domain}`,
    outgoing_smtp_port: 465,
    outgoing_submission_port: 587,
    outgoing_security: "SSL/TLS (or STARTTLS)",
    auth_required: true,
  };

  const copyToClipboard = (text: string, field: string) => {
    navigator.clipboard.writeText(text);
    setCopiedField(field);
    setTimeout(() => setCopiedField(null), 2000);
  };

  if (loading) {
    return (
      <div className="p-8 text-center text-muted-foreground" data-testid="client-config-card">
        {t("common.loading", "Loading email settings...")}
      </div>
    );
  }

  return (
    <div className="space-y-4" data-testid="client-config-card">
      <Card className="border shadow-xs">
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base font-bold flex items-center gap-2">
                <ShieldCheck className="h-5 w-5 text-emerald-600" />
                <span>{t("email.secureConfigTitle", "Secure SSL/TLS Settings (Recommended)")}</span>
              </CardTitle>
              <CardDescription className="text-xs">
                {t(
                  "email.secureConfigSubtitle",
                  "Use these encrypted parameters to configure Outlook, Apple Mail, Thunderbird, or Mobile devices."
                )}
              </CardDescription>
            </div>
            <Badge variant="outline" className="bg-emerald-50 text-emerald-700 border-emerald-300 text-xs">
              TLS Encrypted
            </Badge>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {/* Incoming Server Settings */}
            <div className="p-3.5 bg-muted/40 border rounded-lg space-y-2.5">
              <div className="flex items-center justify-between border-b pb-2">
                <span className="font-bold text-xs flex items-center gap-1.5">
                  <Server className="h-3.5 w-3.5 text-primary" />
                  {t("email.incomingServer", "Incoming Server (IMAP / POP3)")}
                </span>
                <Badge variant="secondary" className="text-[10px] font-mono">
                  {effectiveConfig.incoming_security}
                </Badge>
              </div>

              <div className="space-y-1.5 text-xs">
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.serverHost", "Host")}:</span>
                  <div className="flex items-center gap-1">
                    <span className="font-mono font-semibold" data-testid="incoming-server-val">
                      {effectiveConfig.incoming_server}
                    </span>
                    <button
                      type="button"
                      data-testid="copy-incoming-server"
                      onClick={() => copyToClipboard(effectiveConfig.incoming_server, "inc-host")}
                      className="text-muted-foreground hover:text-foreground"
                    >
                      {copiedField === "inc-host" ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                    </button>
                  </div>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.imapPort", "IMAP Port")}:</span>
                  <span className="font-mono font-semibold" data-testid="imap-port-val">
                    {effectiveConfig.incoming_imap_port}
                  </span>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.pop3Port", "POP3 Port")}:</span>
                  <span className="font-mono font-semibold" data-testid="pop3-port-val">
                    {effectiveConfig.incoming_pop3_port}
                  </span>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.encryption", "Encryption")}:</span>
                  <span className="font-medium text-emerald-600 dark:text-emerald-400">SSL/TLS</span>
                </div>
              </div>
            </div>

            {/* Outgoing Server Settings */}
            <div className="p-3.5 bg-muted/40 border rounded-lg space-y-2.5">
              <div className="flex items-center justify-between border-b pb-2">
                <span className="font-bold text-xs flex items-center gap-1.5">
                  <Mail className="h-3.5 w-3.5 text-primary" />
                  {t("email.outgoingServer", "Outgoing Server (SMTP)")}
                </span>
                <Badge variant="secondary" className="text-[10px] font-mono">
                  {effectiveConfig.outgoing_security}
                </Badge>
              </div>

              <div className="space-y-1.5 text-xs">
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.serverHost", "Host")}:</span>
                  <div className="flex items-center gap-1">
                    <span className="font-mono font-semibold" data-testid="outgoing-server-val">
                      {effectiveConfig.outgoing_server}
                    </span>
                    <button
                      type="button"
                      data-testid="copy-outgoing-server"
                      onClick={() => copyToClipboard(effectiveConfig.outgoing_server, "out-host")}
                      className="text-muted-foreground hover:text-foreground"
                    >
                      {copiedField === "out-host" ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                    </button>
                  </div>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.smtpSslPort", "SMTP SSL Port")}:</span>
                  <span className="font-mono font-semibold" data-testid="smtp-port-val">
                    {effectiveConfig.outgoing_smtp_port} (or {effectiveConfig.outgoing_submission_port})
                  </span>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.submissionPort", "Submission Port")}:</span>
                  <span className="font-mono font-semibold">{effectiveConfig.outgoing_submission_port} (STARTTLS)</span>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("email.smtpAuth", "Authentication")}:</span>
                  <span className="font-medium text-emerald-600 dark:text-emerald-400">Required (Password)</span>
                </div>
              </div>
            </div>
          </div>

          {/* Quick Setup Tips */}
          <div className="p-3 bg-card border rounded-lg flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 text-xs">
            <div className="space-y-0.5">
              <span className="font-semibold block">{t("email.usernameNotice", "Username Format")}</span>
              <p className="text-muted-foreground">
                {t("email.usernameNoticeDesc", "Always use your full email address (e.g., info@{{domain}}) as the username for incoming and outgoing mail.", { domain })}
              </p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <Badge variant="outline" className="flex items-center gap-1">
                <Laptop className="h-3 w-3" />
                Desktop
              </Badge>
              <Badge variant="outline" className="flex items-center gap-1">
                <Smartphone className="h-3 w-3" />
                Mobile
              </Badge>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
