import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { useToast, Button } from "@builtpanel/ui";
import {
  Mail,
  CornerUpRight,
  Clock,
  Settings,
  RefreshCw,
  Globe,
  AlertCircle,
} from "lucide-react";
import {
  Mailbox,
  EmailForwarder,
  Autoresponder,
  EmailClientConfig,
} from "./types";
import { MailboxesTable } from "./MailboxesTable";
import { ForwardersTable } from "./ForwardersTable";
import { AutorespondersTable } from "./AutorespondersTable";
import { ClientConfigCard } from "./ClientConfigCard";

export function ClientEmailPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [domains] = useState<string[]>(["example.com", "mysite.org"]);
  const [selectedDomain, setSelectedDomain] = useState<string>("example.com");
  const [activeTab, setActiveTab] = useState<"mailboxes" | "forwarders" | "autoresponders" | "config">("mailboxes");

  const [mailboxes, setMailboxes] = useState<Mailbox[]>([]);
  const [forwarders, setForwarders] = useState<EmailForwarder[]>([]);
  const [autoresponders, setAutoresponders] = useState<Autoresponder[]>([]);
  const [clientConfig, setClientConfig] = useState<EmailClientConfig | null>(null);

  const [isLoading, setIsLoading] = useState(true);
  const [isError, setIsError] = useState(false);

  const loadData = useCallback(async () => {
    try {
      setIsLoading(true);
      setIsError(false);

      const [mbRes, fwdRes, autoRes, cfgRes] = await Promise.all([
        apiFetch<Mailbox[]>(`/api/v1/email/mailboxes?domain=${selectedDomain}`),
        apiFetch<EmailForwarder[]>(`/api/v1/email/forwarders?domain=${selectedDomain}`),
        apiFetch<Autoresponder[]>(`/api/v1/email/autoresponders?domain=${selectedDomain}`),
        apiFetch<EmailClientConfig>(`/api/v1/email/config?domain=${selectedDomain}`),
      ]);

      setMailboxes(mbRes);
      setForwarders(fwdRes);
      setAutoresponders(autoRes);
      setClientConfig(cfgRes);
    } catch {
      setIsError(true);
    } finally {
      setIsLoading(false);
    }
  }, [selectedDomain]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Handlers for Mailboxes
  const handleCreateMailbox = async (data: {
    username: string;
    domain: string;
    password: string;
    quota_mb: number;
  }) => {
    await apiFetch("/api/v1/email/mailboxes", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("email.mailboxCreatedTitle", "Mailbox created"),
      description: t("email.mailboxCreatedDesc", "Account {{email}} has been provisioned.", {
        email: `${data.username}@${data.domain}`,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleDeleteMailbox = async (id: string) => {
    await apiFetch(`/api/v1/email/mailboxes/${id}`, {
      method: "DELETE",
    });
    toast({
      title: t("email.mailboxDeletedTitle", "Mailbox deleted"),
      description: t("email.mailboxDeletedDesc", "Mailbox and storage have been removed."),
      variant: "success",
    });
    loadData();
  };

  const handleChangePassword = async (id: string, newPass: string) => {
    await apiFetch(`/api/v1/email/mailboxes/${id}/password`, {
      method: "POST",
      body: JSON.stringify({ password: newPass }),
    });
    toast({
      title: t("email.passwordChangedTitle", "Password updated"),
      description: t("email.passwordChangedDesc", "Mailbox password has been updated successfully."),
      variant: "success",
    });
  };

  // Handlers for Forwarders
  const handleCreateForwarder = async (data: {
    source: string;
    destination: string;
    domain: string;
  }) => {
    await apiFetch("/api/v1/email/forwarders", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("email.forwarderCreatedTitle", "Forwarder added"),
      description: t("email.forwarderCreatedDesc", "Forwarder from {{source}} created.", {
        source: data.source,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleDeleteForwarder = async (id: string) => {
    await apiFetch(`/api/v1/email/forwarders/${id}`, {
      method: "DELETE",
    });
    toast({
      title: t("email.forwarderDeletedTitle", "Forwarder removed"),
      description: t("email.forwarderDeletedDesc", "Email forwarder has been deleted."),
      variant: "success",
    });
    loadData();
  };

  // Handlers for Autoresponders
  const handleCreateAutoresponder = async (data: {
    email: string;
    domain: string;
    subject: string;
    body: string;
    start_date?: string;
    end_date?: string;
  }) => {
    await apiFetch("/api/v1/email/autoresponders", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("email.autoresponderCreatedTitle", "Autoresponder configured"),
      description: t("email.autoresponderCreatedDesc", "Auto-reply is now active for {{email}}.", {
        email: data.email,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleDeleteAutoresponder = async (id: string) => {
    await apiFetch(`/api/v1/email/autoresponders/${id}`, {
      method: "DELETE",
    });
    toast({
      title: t("email.autoresponderDeletedTitle", "Autoresponder removed"),
      description: t("email.autoresponderDeletedDesc", "Autoresponder has been deleted."),
      variant: "success",
    });
    loadData();
  };

  return (
    <div className="space-y-6" data-testid="client-email-page">
      {/* Header */}
      <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="email-page-title">
            {t("email.pageTitle", "Email Management")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t(
              "email.pageSubtitle",
              "Manage email mailboxes, forwarders, autoresponders, and device configuration."
            )}
          </p>
        </div>

        {/* Controls: Domain selector & Refresh */}
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 border rounded-md px-3 py-1.5 bg-background shadow-sm">
            <Globe className="h-4 w-4 text-muted-foreground shrink-0" />
            <select
              value={selectedDomain}
              onChange={(e) => setSelectedDomain(e.target.value)}
              className="bg-transparent text-sm font-medium focus:outline-none"
              data-testid="email-domain-selector"
            >
              {domains.map((dom) => (
                <option key={dom} value={dom}>
                  @{dom}
                </option>
              ))}
            </select>
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={loadData}
            disabled={isLoading}
            className="gap-2"
            data-testid="refresh-email-btn"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Error state */}
      {isError && (
        <div className="p-4 rounded-lg bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 border border-red-200 dark:border-red-900 flex items-center gap-3">
          <AlertCircle className="h-5 w-5 shrink-0" />
          <div className="flex-1 text-sm">
            {t("email.loadError", "Failed to load email services. Please check connection.")}
          </div>
          <Button variant="outline" size="sm" onClick={loadData}>
            {t("common.retry", "Retry")}
          </Button>
        </div>
      )}

      {/* Navigation Tabs */}
      <div className="border-b flex gap-4">
        <button
          onClick={() => setActiveTab("mailboxes")}
          data-testid="tab-mailboxes"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "mailboxes"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Mail className="h-4 w-4" />
          {t("email.mailboxesTab", "Email Accounts")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs">
            {mailboxes.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("forwarders")}
          data-testid="tab-forwarders"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "forwarders"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <CornerUpRight className="h-4 w-4" />
          {t("email.forwardersTab", "Forwarders")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs">
            {forwarders.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("autoresponders")}
          data-testid="tab-autoresponders"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "autoresponders"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Clock className="h-4 w-4" />
          {t("email.autorespondersTab", "Autoresponders")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs">
            {autoresponders.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("config")}
          data-testid="tab-config"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "config"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Settings className="h-4 w-4" />
          {t("email.configTab", "Connect Devices (IMAP/SMTP)")}
        </button>
      </div>

      {/* Tab Content */}
      <div className="pt-2">
        {activeTab === "mailboxes" && (
          <MailboxesTable
            mailboxes={mailboxes}
            domain={selectedDomain}
            loading={isLoading}
            onCreateMailbox={handleCreateMailbox}
            onDeleteMailbox={handleDeleteMailbox}
            onChangePassword={handleChangePassword}
          />
        )}

        {activeTab === "forwarders" && (
          <ForwardersTable
            forwarders={forwarders}
            domain={selectedDomain}
            loading={isLoading}
            onCreateForwarder={handleCreateForwarder}
            onDeleteForwarder={handleDeleteForwarder}
          />
        )}

        {activeTab === "autoresponders" && (
          <AutorespondersTable
            autoresponders={autoresponders}
            domain={selectedDomain}
            loading={isLoading}
            onCreateAutoresponder={handleCreateAutoresponder}
            onDeleteAutoresponder={handleDeleteAutoresponder}
          />
        )}

        {activeTab === "config" && (
          <ClientConfigCard
            config={clientConfig}
            domain={selectedDomain}
            loading={isLoading}
          />
        )}
      </div>
    </div>
  );
}
