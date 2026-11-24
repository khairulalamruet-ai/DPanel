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
import { Package } from "lucide-react";

interface CreateResellerPackageDialogProps {
  open: boolean;
  onClose: () => void;
  onCreate: (data: {
    name: string;
    max_accounts: number;
    max_disk_gb: number;
    max_bandwidth_gb: number;
    allow_overselling: boolean;
  }) => Promise<void>;
}

export function CreateResellerPackageDialog({
  open,
  onClose,
  onCreate,
}: CreateResellerPackageDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("");
  const [maxAccounts, setMaxAccounts] = useState(25);
  const [maxDiskGb, setMaxDiskGb] = useState(100);
  const [maxBwGb, setMaxBwGb] = useState(1000);
  const [allowOverselling, setAllowOverselling] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    if (!name.trim()) {
      setErrorMsg("Please enter a package name");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        name: name.trim(),
        max_accounts: Number(maxAccounts) || 25,
        max_disk_gb: Number(maxDiskGb) || 100,
        max_bandwidth_gb: Number(maxBwGb) || 1000,
        allow_overselling: allowOverselling,
      });
      setName("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create package");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-reseller-package-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Package className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-package-title">
                {t("reseller.createPackageTitle", "Create Reseller Package")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("reseller.createPackageSubtitle", "Define resource allocations and overselling rules for resellers.")}
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
              {t("reseller.packageNameLabel", "Package Name")}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Agency Pro Reseller"
              required
              data-testid="input-package-name"
            />
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.maxAccountsLabel", "Accounts")}
              </label>
              <Input
                type="number"
                min="1"
                value={maxAccounts}
                onChange={(e) => setMaxAccounts(Number(e.target.value))}
                required
                data-testid="input-package-accounts"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.diskGbLabel", "Disk (GB)")}
              </label>
              <Input
                type="number"
                min="1"
                value={maxDiskGb}
                onChange={(e) => setMaxDiskGb(Number(e.target.value))}
                required
                data-testid="input-package-disk"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.bwGbLabel", "BW (GB)")}
              </label>
              <Input
                type="number"
                min="1"
                value={maxBwGb}
                onChange={(e) => setMaxBwGb(Number(e.target.value))}
                required
                data-testid="input-package-bw"
              />
            </div>
          </div>

          <div className="flex items-center gap-2 pt-2">
            <input
              type="checkbox"
              id="overselling-checkbox"
              checked={allowOverselling}
              onChange={(e) => setAllowOverselling(e.target.checked)}
              className="rounded border-gray-300 text-primary focus:ring-primary h-4 w-4"
              data-testid="input-package-overselling"
            />
            <label htmlFor="overselling-checkbox" className="text-xs font-medium cursor-pointer">
              {t("reseller.allowOversellingLabel", "Allow Resource Overselling")}
            </label>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-reseller-package">
              {isSubmitting ? t("common.saving", "Saving...") : t("reseller.createPackageBtn", "Save Package")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
