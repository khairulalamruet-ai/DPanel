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
import { CornerUpRight } from "lucide-react";

interface CreateForwarderDialogProps {
  open: boolean;
  domain: string;
  onClose: () => void;
  onCreate: (data: { source: string; destination: string; domain: string }) => Promise<void>;
}

export function CreateForwarderDialog({
  open,
  domain,
  onClose,
  onCreate,
}: CreateForwarderDialogProps) {
  const { t } = useTranslation();

  const [sourceUser, setSourceUser] = useState("");
  const [destination, setDestination] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    const cleanSource = sourceUser.trim().toLowerCase().replace(/@.*$/, "");
    const cleanDest = destination.trim().toLowerCase();

    if (!cleanSource) {
      setErrorMsg("Please enter the source email username");
      return;
    }
    if (!cleanDest || !cleanDest.includes("@")) {
      setErrorMsg("Please enter a valid forward-to destination email address");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        source: `${cleanSource}@${domain}`,
        destination: cleanDest,
        domain,
      });
      setSourceUser("");
      setDestination("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create forwarder");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-forwarder-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <CornerUpRight className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-forwarder-title">
                {t("email.createForwarderTitle", "Add Email Forwarder")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("email.createForwarderSubtitle", "Forward copies of incoming mail to an external or internal address.")}
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
              {t("email.forwarderSourceLabel", "Address to Forward")}
            </label>
            <div className="flex items-center gap-2">
              <Input
                value={sourceUser}
                onChange={(e) => setSourceUser(e.target.value)}
                placeholder="sales"
                required
                className="flex-1"
                data-testid="forwarder-source-input"
              />
              <span className="text-sm font-mono text-muted-foreground">@{domain}</span>
            </div>
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("email.forwarderDestinationLabel", "Forward To (Destination)")}
            </label>
            <Input
              type="email"
              value={destination}
              onChange={(e) => setDestination(e.target.value)}
              placeholder="team@example.com"
              required
              data-testid="forwarder-destination-input"
            />
            <p className="text-[11px] text-muted-foreground mt-1">
              {t("email.forwarderDestinationHint", "Incoming emails will automatically be forwarded to this address.")}
            </p>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="submit-forwarder-btn">
              {isSubmitting ? t("common.saving", "Saving...") : t("email.addForwarderBtn", "Add Forwarder")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
