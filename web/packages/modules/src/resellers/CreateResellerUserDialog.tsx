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
import { UserPlus, RefreshCw } from "lucide-react";
import { ResellerPlan } from "./types";

interface CreateResellerUserDialogProps {
  open: boolean;
  plans: ResellerPlan[];
  onClose: () => void;
  onCreate: (data: {
    username: string;
    email: string;
    domain: string;
    password: string;
    plan_name: string;
  }) => Promise<void>;
}

export function CreateResellerUserDialog({
  open,
  plans,
  onClose,
  onCreate,
}: CreateResellerUserDialogProps) {
  const { t } = useTranslation();

  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [domain, setDomain] = useState("");
  const [password, setPassword] = useState("ClientSec!2026");
  const [selectedPlan, setSelectedPlan] = useState<string>(plans[0]?.name || "Starter Plan");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const generatePassword = () => {
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

    const cleanUser = username.trim().toLowerCase();
    if (!cleanUser) {
      setErrorMsg("Please enter a username");
      return;
    }
    if (!domain.trim()) {
      setErrorMsg("Please enter a primary domain");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        username: cleanUser,
        email: email.trim() || `${cleanUser}@${domain.trim()}`,
        domain: domain.trim().toLowerCase(),
        password,
        plan_name: selectedPlan,
      });
      setUsername("");
      setEmail("");
      setDomain("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create user");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-reseller-user-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <UserPlus className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-reseller-user-title">
                {t("reseller.createUserTitle", "Create Client Account")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("reseller.createUserSubtitle", "Provision a new tenant web hosting account under your reseller quota.")}
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
              {t("reseller.usernameLabel", "Username")}
            </label>
            <Input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="e.g. acme_corp"
              required
              data-testid="input-reseller-user-username"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.domainLabel", "Primary Domain")}
            </label>
            <Input
              value={domain}
              onChange={(e) => setDomain(e.target.value)}
              placeholder="acmecorp.com"
              required
              data-testid="input-reseller-user-domain"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.emailLabel", "Owner Contact Email")}
            </label>
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="admin@acmecorp.com"
              data-testid="input-reseller-user-email"
            />
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-xs font-semibold">
                {t("reseller.passwordLabel", "Account Password")}
              </label>
              <button
                type="button"
                onClick={generatePassword}
                className="text-xs text-primary hover:underline flex items-center gap-1"
                data-testid="btn-generate-reseller-user-pass"
              >
                <RefreshCw className="h-3 w-3" />
                {t("common.generate", "Generate")}
              </button>
            </div>
            <Input
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              data-testid="input-reseller-user-password"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.planLabel", "Hosting Plan")}
            </label>
            <select
              value={selectedPlan}
              onChange={(e) => setSelectedPlan(e.target.value)}
              className="w-full border rounded-md px-3 py-2 text-sm bg-background"
              data-testid="select-reseller-user-plan"
            >
              {plans.length > 0 ? (
                plans.map((p) => (
                  <option key={p.id} value={p.name}>
                    {p.name} ({Math.round(p.disk_mb / 1024 * 10) / 10} GB disk)
                  </option>
                ))
              ) : (
                <option value="Default Reseller Plan">Default Reseller Plan</option>
              )}
            </select>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-reseller-user">
              {isSubmitting ? t("common.creating", "Creating...") : t("reseller.createUserBtn", "Create Account")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
