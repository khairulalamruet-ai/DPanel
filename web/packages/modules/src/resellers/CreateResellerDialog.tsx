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
import { Briefcase, RefreshCw } from "lucide-react";
import { ResellerPackage } from "./types";

interface CreateResellerDialogProps {
  open: boolean;
  packages: ResellerPackage[];
  onClose: () => void;
  onCreate: (data: {
    username: string;
    email: string;
    company_name: string;
    password: string;
    package_id: string;
  }) => Promise<void>;
}

export function CreateResellerDialog({
  open,
  packages,
  onClose,
  onCreate,
}: CreateResellerDialogProps) {
  const { t } = useTranslation();

  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [companyName, setCompanyName] = useState("");
  const [password, setPassword] = useState("ResellerMaster!2026");
  const [packageId, setPackageId] = useState(packages[0]?.id || "");
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
    if (!companyName.trim()) {
      setErrorMsg("Please enter a company or brand name");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        username: cleanUser,
        email: email.trim(),
        company_name: companyName.trim(),
        password,
        package_id: packageId || packages[0]?.id || "pkg-1",
      });
      setUsername("");
      setEmail("");
      setCompanyName("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create reseller");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-reseller-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Briefcase className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-reseller-title">
                {t("reseller.createResellerTitle", "Provision New Reseller")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("reseller.createResellerSubtitle", "Grant partner or reseller hosting rights with defined quota tier.")}
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
              {t("reseller.companyNameLabel", "Company / Agency Name")}
            </label>
            <Input
              value={companyName}
              onChange={(e) => setCompanyName(e.target.value)}
              placeholder="e.g. Acme Web Agency"
              required
              data-testid="input-reseller-company"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.usernameLabel", "Username")}
            </label>
            <Input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="acme_reseller"
              required
              data-testid="input-reseller-username"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.emailLabel", "Contact Email")}
            </label>
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="owner@acmeweb.com"
              required
              data-testid="input-reseller-email"
            />
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-xs font-semibold">
                {t("reseller.passwordLabel", "Password")}
              </label>
              <button
                type="button"
                onClick={generatePassword}
                className="text-xs text-primary hover:underline flex items-center gap-1"
                data-testid="btn-generate-reseller-pass"
              >
                <RefreshCw className="h-3 w-3" />
                {t("common.generate", "Generate")}
              </button>
            </div>
            <Input
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              data-testid="input-reseller-password"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("reseller.packageLabel", "Assigned Package")}
            </label>
            <select
              value={packageId}
              onChange={(e) => setPackageId(e.target.value)}
              className="w-full border rounded-md px-3 py-2 text-sm bg-background"
              data-testid="select-reseller-package"
            >
              {packages.map((pkg) => (
                <option key={pkg.id} value={pkg.id}>
                  {pkg.name} ({pkg.max_accounts} accts, {pkg.max_disk_gb} GB disk)
                </option>
              ))}
            </select>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-reseller">
              {isSubmitting ? t("common.creating", "Creating...") : t("reseller.createResellerBtn", "Provision Reseller")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
