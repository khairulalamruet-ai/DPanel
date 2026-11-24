import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { Key, Copy, Check, ShieldAlert, Sparkles } from "lucide-react";
import { TokenScope, CreateTokenResponse } from "./types";

interface CreateTokenDialogProps {
  open: boolean;
  onClose: () => void;
  onCreated: (res: CreateTokenResponse) => void;
}

const AVAILABLE_SCOPES: Array<{
  id: TokenScope;
  title: string;
  desc: string;
  category: string;
}> = [
  {
    id: "users:read",
    title: "Read Users & Tenants (GET /users)",
    desc: "Query tenant accounts, email addresses, and subscription statuses.",
    category: "Users",
  },
  {
    id: "users:write",
    title: "Manage Users (POST/PUT/DELETE /users)",
    desc: "Create new tenants, change account passwords, or suspend users.",
    category: "Users",
  },
  {
    id: "domains:read",
    title: "Read Domains & Vhosts (GET /domains)",
    desc: "List hosted domains, document roots, and SSL issuance status.",
    category: "Domains",
  },
  {
    id: "domains:write",
    title: "Provision Domains (POST /domains)",
    desc: "Add new domains, configure vhosts, and trigger Let's Encrypt certificates.",
    category: "Domains",
  },
  {
    id: "billing:whmcs",
    title: "WHMCS & Billing Automation (/billing/*)",
    desc: "Allow WHMCS automation server to provision, suspend, unsuspend, and terminate hosting accounts.",
    category: "Billing",
  },
  {
    id: "dns:manage",
    title: "DNS Zone Records (/dns/*)",
    desc: "Create and update A, CNAME, MX, and TXT records programmatically.",
    category: "DNS",
  },
];

export function CreateTokenDialog({ open, onClose, onCreated }: CreateTokenDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<TokenScope[]>(["users:read"]);
  const [expiryDays, setExpiryDays] = useState<string>("30");
  const [isLoading, setIsLoading] = useState(false);
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const handleToggleScope = (scope: TokenScope) => {
    setScopes((prev) =>
      prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope]
    );
  };

  const handleSelectAll = () => {
    setScopes(AVAILABLE_SCOPES.map((s) => s.id));
  };

  const handleClearAll = () => {
    setScopes([]);
  };

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (scopes.length === 0) {
      toast({
        title: t("apiTokens.noScopeError", "Scope Required"),
        description: t("apiTokens.noScopeErrorDesc", "Select at least one API scope for this token."),
        variant: "destructive",
      });
      return;
    }

    setIsLoading(true);
    try {
      const res = await apiFetch<CreateTokenResponse>("/api/v1/tokens", {
        method: "POST",
        body: JSON.stringify({
          name: name.trim(),
          scopes,
          expiry_days: expiryDays === "never" ? null : parseInt(expiryDays, 10),
        }),
      });

      setCreatedSecret(res.secret_key);
      onCreated(res);
      toast({
        title: t("apiTokens.createdSuccess", "API Token Generated"),
        description: t("apiTokens.createdSuccessDesc", "Copy the secret key now before closing this dialog."),
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || "Failed to create API token",
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  const handleCopy = () => {
    if (!createdSecret) return;
    navigator.clipboard.writeText(createdSecret);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleFinish = () => {
    setCreatedSecret(null);
    setName("");
    setScopes(["users:read"]);
    setExpiryDays("30");
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && handleFinish()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Key className="h-5 w-5 text-primary" />
            <span>{t("apiTokens.createTitle", "Generate New API Token")}</span>
          </DialogTitle>
          <DialogDescription className="text-xs">
            {t(
              "apiTokens.createDesc",
              "Scoped Bearer tokens authenticate third-party billing software (WHMCS), CLI scripts, and custom integrations."
            )}
          </DialogDescription>
        </DialogHeader>

        {createdSecret ? (
          <div className="space-y-4 py-3" data-testid="token-secret-revealed-view">
            <div className="p-3.5 rounded-lg bg-amber-500/10 border border-amber-500/30 text-amber-800 dark:text-amber-200 text-xs flex items-start gap-2.5">
              <ShieldAlert className="h-4 w-4 text-amber-500 shrink-0 mt-0.5" />
              <div>
                <span className="font-semibold">{t("apiTokens.saveSecretNotice", "Copy your API Secret Key now")}</span>
                <p className="mt-0.5 leading-relaxed">
                  {t(
                    "apiTokens.saveSecretWarning",
                    "For your security, this key is encrypted with Argon2id and will never be displayed again."
                  )}
                </p>
              </div>
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-semibold text-foreground">
                {t("apiTokens.secretKeyLabel", "Secret Token")}
              </label>
              <div className="flex items-center gap-2">
                <Input
                  readOnly
                  value={createdSecret}
                  className="font-mono text-xs bg-muted/40 select-all"
                  data-testid="created-secret-input"
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={handleCopy}
                  className="gap-1.5 shrink-0 text-xs h-9"
                  data-testid="btn-copy-secret-key"
                >
                  {copied ? <Check className="h-4 w-4 text-emerald-500" /> : <Copy className="h-4 w-4" />}
                  <span>{copied ? t("common.copied", "Copied") : t("common.copy", "Copy")}</span>
                </Button>
              </div>
            </div>

            <DialogFooter>
              <Button onClick={handleFinish} className="text-xs" data-testid="btn-token-done">
                {t("common.done", "Done")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={handleCreate} className="space-y-4 py-2">
            <div className="space-y-1.5">
              <label className="text-xs font-semibold text-foreground">
                {t("apiTokens.nameLabel", "Token Name / Description")}
              </label>
              <Input
                placeholder="e.g. WHMCS Production Billing Gateway"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                className="text-xs"
                data-testid="input-token-name"
              />
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <label className="text-xs font-semibold text-foreground">
                  {t("apiTokens.expirationLabel", "Expiration Period")}
                </label>
                <select
                  value={expiryDays}
                  onChange={(e) => setExpiryDays(e.target.value)}
                  className="w-full text-xs h-9 px-3 rounded-md border border-input bg-background"
                  data-testid="select-token-expiry"
                >
                  <option value="7">7 Days</option>
                  <option value="30">30 Days</option>
                  <option value="90">90 Days</option>
                  <option value="365">1 Year</option>
                  <option value="never">Never (Persistent Service)</option>
                </select>
              </div>

              <div className="space-y-1.5 flex flex-col justify-end">
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={handleSelectAll}
                    className="text-[11px] h-7 px-2"
                  >
                    Select All
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={handleClearAll}
                    className="text-[11px] h-7 px-2 text-muted-foreground"
                  >
                    Clear All
                  </Button>
                </div>
              </div>
            </div>

            {/* Scope selection list */}
            <div className="space-y-2 pt-1">
              <label className="text-xs font-semibold text-foreground flex items-center justify-between">
                <span>{t("apiTokens.scopesLabel", "Authorized API Scopes")}</span>
                <span className="text-muted-foreground font-normal text-[11px]">
                  {scopes.length} selected
                </span>
              </label>

              <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                {AVAILABLE_SCOPES.map((scope) => {
                  const checked = scopes.includes(scope.id);
                  return (
                    <div
                      key={scope.id}
                      onClick={() => handleToggleScope(scope.id)}
                      className={`flex items-start gap-3 p-2.5 rounded-lg border text-xs cursor-pointer transition-colors ${
                        checked
                          ? "bg-primary/5 border-primary/30"
                          : "bg-card border-border hover:bg-muted/30"
                      }`}
                      data-testid={`scope-item-${scope.id}`}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={() => {}}
                        className="mt-0.5 rounded border-border text-primary focus:ring-0"
                      />
                      <div className="space-y-0.5 flex-1 min-w-0">
                        <div className="flex items-center justify-between gap-2">
                          <span className="font-semibold text-foreground">{scope.title}</span>
                          <Badge variant="outline" className="text-[10px] font-mono">
                            {scope.category}
                          </Badge>
                        </div>
                        <p className="text-[11px] text-muted-foreground">{scope.desc}</p>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            <DialogFooter className="pt-2">
              <Button type="button" variant="outline" onClick={onClose} className="text-xs">
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                type="submit"
                disabled={isLoading || !name.trim()}
                className="gap-2 text-xs"
                data-testid="btn-submit-create-token"
              >
                <Sparkles className="h-4 w-4" />
                <span>{isLoading ? t("common.creating", "Generating...") : t("apiTokens.generateButton", "Generate Token")}</span>
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
