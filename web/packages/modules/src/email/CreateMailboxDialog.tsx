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
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Mail, RefreshCw, Key, HardDrive } from "lucide-react";

interface CreateMailboxDialogProps {
  open: boolean;
  domain: string;
  onClose: () => void;
  onCreate: (data: { username: string; domain: string; password: string; quota_mb: number }) => Promise<void>;
}

export function CreateMailboxDialog({
  open,
  domain,
  onClose,
  onCreate,
}: CreateMailboxDialogProps) {
  const { t } = useTranslation();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("SecureMail!2026");
  const [quotaMb, setQuotaMb] = useState(1024);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleGeneratePassword = () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
    let pass = "";
    for (let i = 0; i < 16; i++) {
      pass += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    setPassword(pass);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    const cleanUsername = username.trim().toLowerCase().replace(/@.*$/, "");
    if (!cleanUsername) {
      setErrorMsg("Please enter a valid mailbox username");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        username: cleanUsername,
        domain,
        password,
        quota_mb: Number(quotaMb) || 1024,
      });
      setUsername("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create mailbox");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-mailbox-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Mail className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-mailbox-title">
                {t("email.createMailboxTitle", "Create Email Account")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("email.createMailboxSubtitle", "Add a new mailbox on @{{domain}} with custom storage quota.", { domain })}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {errorMsg && (
            <div className="p-3 bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300 rounded-lg text-xs">
              {errorMsg}
            </div>
          )}

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("email.emailAddressLabel", "Email Address")}
            </label>
            <div className="flex items-center">
              <Input
                type="text"
                placeholder="e.g. info or sales"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="rounded-r-none text-xs h-8 font-mono"
                required
                data-testid="mailbox-username-input"
              />
              <span className="bg-muted px-3 py-1 text-xs border border-l-0 rounded-r font-mono text-muted-foreground whitespace-nowrap">
                @{domain}
              </span>
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-xs font-semibold">
                {t("email.passwordLabel", "Password")}
              </label>
              <button
                type="button"
                onClick={handleGeneratePassword}
                className="text-[10px] text-primary hover:underline flex items-center gap-0.5"
                data-testid="btn-generate-mail-password"
              >
                <RefreshCw className="h-2.5 w-2.5" />
                {t("common.generate", "Generate")}
              </button>
            </div>
            <Input
              type="text"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="text-xs h-8 font-mono"
              required
              data-testid="mailbox-password-input"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("email.quotaLabel", "Storage Quota (MB)")}
            </label>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min="100"
                max="51200"
                value={quotaMb}
                onChange={(e) => setQuotaMb(Number(e.target.value))}
                className="text-xs h-8 font-mono w-32"
                required
                data-testid="mailbox-quota-input"
              />
              <span className="text-xs text-muted-foreground">MB ({Math.round(quotaMb / 1024 * 10) / 10} GB)</span>
            </div>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="submit-mailbox-btn">
              {isSubmitting ? t("common.creating", "Creating...") : t("email.createAccountBtn", "Create Account")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
