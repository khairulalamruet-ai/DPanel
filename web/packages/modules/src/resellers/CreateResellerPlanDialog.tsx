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
import { Layers } from "lucide-react";

interface CreateResellerPlanDialogProps {
  open: boolean;
  onClose: () => void;
  onCreate: (data: {
    name: string;
    disk_mb: number;
    bandwidth_mb: number;
    domains_limit: number;
    databases_limit: number;
    mailboxes_limit: number;
  }) => Promise<void>;
}

export function CreateResellerPlanDialog({
  open,
  onClose,
  onCreate,
}: CreateResellerPlanDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("");
  const [diskGb, setDiskGb] = useState(10);
  const [bwGb, setBwGb] = useState(100);
  const [domainsLimit, setDomainsLimit] = useState(5);
  const [databasesLimit, setDatabasesLimit] = useState(5);
  const [mailboxesLimit, setMailboxesLimit] = useState(10);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    if (!name.trim()) {
      setErrorMsg("Please enter a plan name");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        name: name.trim(),
        disk_mb: (Number(diskGb) || 10) * 1024,
        bandwidth_mb: (Number(bwGb) || 100) * 1024,
        domains_limit: Number(domainsLimit) || 1,
        databases_limit: Number(databasesLimit) || 1,
        mailboxes_limit: Number(mailboxesLimit) || 1,
      });
      setName("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create plan");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-reseller-plan-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Layers className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-reseller-plan-title">
                {t("reseller.createPlanTitle", "Create Custom Hosting Plan")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("reseller.createPlanSubtitle", "Define disk quotas and limits for your client accounts.")}
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
              {t("reseller.planNameLabel", "Plan Name")}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Standard Web Package"
              required
              data-testid="input-reseller-plan-name"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.diskGbLabel", "Disk Space (GB)")}
              </label>
              <Input
                type="number"
                min="1"
                value={diskGb}
                onChange={(e) => setDiskGb(Number(e.target.value))}
                required
                data-testid="input-reseller-plan-disk"
              />
            </div>
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.bwGbLabel", "Bandwidth (GB)")}
              </label>
              <Input
                type="number"
                min="1"
                value={bwGb}
                onChange={(e) => setBwGb(Number(e.target.value))}
                required
                data-testid="input-reseller-plan-bw"
              />
            </div>
          </div>

          <div className="grid grid-cols-3 gap-2">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.domainsLabel", "Domains")}
              </label>
              <Input
                type="number"
                min="1"
                value={domainsLimit}
                onChange={(e) => setDomainsLimit(Number(e.target.value))}
                required
              />
            </div>
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.dbsLabel", "MySQL DBs")}
              </label>
              <Input
                type="number"
                min="0"
                value={databasesLimit}
                onChange={(e) => setDatabasesLimit(Number(e.target.value))}
                required
              />
            </div>
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.mailboxesLabel", "Mailboxes")}
              </label>
              <Input
                type="number"
                min="0"
                value={mailboxesLimit}
                onChange={(e) => setMailboxesLimit(Number(e.target.value))}
                required
              />
            </div>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-reseller-plan">
              {isSubmitting ? t("common.saving", "Saving...") : t("reseller.createPlanBtn", "Create Plan")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
