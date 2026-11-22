import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { createFtp } from "./api";
import { UserPlus, RefreshCw } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface CreateFtpAccountDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** What every account name starts with (`alice_`). */
  prefix?: string;
  onCreated: () => void;
}

export function CreateFtpAccountDialog({
  open,
  onOpenChange,
  prefix = "",
  onCreated,
}: CreateFtpAccountDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [usernameSuffix, setUsernameSuffix] = useState("");
  const [password, setPassword] = useState("");
  const [homeDir, setHomeDir] = useState("");
  const [quotaUnlimited, setQuotaUnlimited] = useState(true);
  const [quotaMb, setQuotaMb] = useState<number>(500);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const generatePassword = () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
    let generated = "";
    for (let i = 0; i < 16; i++) {
      generated += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    setPassword(generated);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanSuffix = usernameSuffix.trim().toLowerCase();
    if (!cleanSuffix || !password) return;

    const fullUsername = `${prefix}${cleanSuffix}`;

    try {
      setIsSubmitting(true);
      await createFtp({
        name: fullUsername,
        password,
        home: homeDir.trim().replace(/^\/+/, ""),
        quotaMb: quotaUnlimited ? null : Number(quotaMb),
      });

      toast({
        title: t("common.success"),
        description: t("ftp.accountCreated", { username: fullUsername }),
      });

      setUsernameSuffix("");
      setPassword("");
      onOpenChange(false);
      onCreated();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <UserPlus className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="create-ftp-dialog-title">
              {t("ftp.newAccount")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {/* Username */}
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("ftp.username")}
            </label>
            <div className="flex items-center">
              <span className="inline-flex items-center px-3 rounded-l-md border border-r-0 border-input bg-muted text-muted-foreground text-sm font-mono h-9">
                {prefix}
              </span>
              <Input
                value={usernameSuffix}
                onChange={(e) => setUsernameSuffix(e.target.value)}
                placeholder="dev"
                className="rounded-l-none font-mono"
                autoFocus
                data-testid="input-ftp-username"
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {t("ftp.fullUsername")}: {prefix}{usernameSuffix || "..."}
            </p>
          </div>

          {/* Password */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-sm font-medium">
                {t("ftp.password")}
              </label>
              <button
                type="button"
                onClick={generatePassword}
                className="text-xs text-primary hover:underline flex items-center gap-1"
                data-testid="btn-generate-ftp-pw"
              >
                <RefreshCw className="w-3 h-3" />
                <span>{t("ftp.generatePassword")}</span>
              </button>
            </div>
            <Input
              type="text"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••••••"
              className="font-mono text-xs"
              data-testid="input-ftp-password"
            />
          </div>

          {/* Home Directory */}
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("ftp.homeDirectory")}
            </label>
            <Input
              value={homeDir}
              onChange={(e) => setHomeDir(e.target.value)}
              placeholder="public_html"
              className="font-mono text-xs"
              data-testid="input-ftp-homedir"
            />
            <p className="text-xs text-muted-foreground">
              {t("ftp.homeDirectoryDescription")}
            </p>
          </div>

          {/* Quota */}
          <div className="space-y-2">
            <label className="text-sm font-medium">
              {t("ftp.quota")}
            </label>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="quota"
                  checked={quotaUnlimited}
                  onChange={() => setQuotaUnlimited(true)}
                  data-testid="radio-quota-unlimited"
                />
                <span>{t("ftp.unlimited")}</span>
              </label>
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="quota"
                  checked={!quotaUnlimited}
                  onChange={() => setQuotaUnlimited(false)}
                  data-testid="radio-quota-limited"
                />
                <span>{t("ftp.customQuota")}</span>
              </label>
            </div>

            {!quotaUnlimited && (
              <div className="flex items-center gap-2 pt-1">
                <Input
                  type="number"
                  min="1"
                  value={quotaMb}
                  onChange={(e) => setQuotaMb(Number(e.target.value))}
                  className="w-32 font-mono"
                  data-testid="input-ftp-quota-mb"
                />
                <span className="text-sm text-muted-foreground">MB</span>
              </div>
            )}
          </div>

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={!usernameSuffix.trim() || !password || isSubmitting}
              data-testid="btn-submit-create-ftp"
            >
              {isSubmitting ? t("common.submitting") : t("common.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
