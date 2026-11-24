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
import { LifeBuoy, AlertTriangle, CheckCircle2, ShieldAlert } from "lucide-react";
import { RecoveryRequestPayload, RecoveryRequestResponse } from "./types";

interface RecoveryRequestDialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit: (payload: RecoveryRequestPayload) => Promise<RecoveryRequestResponse>;
}

export function RecoveryRequestDialog({ open, onClose, onSubmit }: RecoveryRequestDialogProps) {
  const { t } = useTranslation();

  const [reason, setReason] = useState("hardware_migration");
  const [email, setEmail] = useState("");
  const [notes, setNotes] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [result, setResult] = useState<RecoveryRequestResponse | null>(null);
  const [errorMsg, setErrorMsg] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!email.trim()) {
      setErrorMsg(t("license.emailRequired", "Contact email is required."));
      return;
    }

    setErrorMsg("");
    setIsSubmitting(true);
    try {
      const res = await onSubmit({
        reason,
        contact_email: email.trim().toLowerCase(),
        notes: notes.trim(),
      });
      setResult(res);
    } catch (err: any) {
      setErrorMsg(err.message || t("license.recoverySubmitError", "Failed to submit recovery request."));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleModalClose = () => {
    setResult(null);
    setErrorMsg("");
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && handleModalClose()}>
      <DialogContent data-testid="recovery-request-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <LifeBuoy className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="recovery-dialog-title">
                {t("license.recoveryModalTitle", "License Authority Recovery Request")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "license.recoveryModalSubtitle",
                  "Submit an official recovery request to clear tampering flags or re-bind to new hardware."
                )}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {result ? (
          <div className="py-4 space-y-4" data-testid="recovery-success-view">
            <div className="p-4 rounded-lg bg-emerald-500/10 border border-emerald-500/30 text-emerald-800 dark:text-emerald-300 space-y-2">
              <div className="flex items-center gap-2 font-bold text-sm">
                <CheckCircle2 className="h-5 w-5 text-emerald-500 shrink-0" />
                <span>{t("license.recoverySubmitted", "Recovery Ticket Dispatched")}</span>
              </div>
              <p className="text-xs">{result.message}</p>
              <div className="pt-2 font-mono text-xs">
                <span className="text-muted-foreground">Ticket ID: </span>
                <span className="font-bold text-foreground" data-testid="text-recovery-ticket-id">
                  {result.ticket_id}
                </span>
              </div>
            </div>

            <p className="text-xs text-muted-foreground">
              {t(
                "license.recoveryNextSteps",
                "The License Authority engineering team will verify the integrity audit log. You will receive updates via email."
              )}
            </p>

            <DialogFooter className="pt-2">
              <Button onClick={handleModalClose} data-testid="btn-close-recovery-success">
                {t("common.close", "Close")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4 py-2">
            {errorMsg && (
              <div className="p-3 bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 rounded-lg text-xs">
                {errorMsg}
              </div>
            )}

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("license.recoveryReasonLabel", "Incident Reason")}
              </label>
              <select
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                className="w-full border rounded-md px-3 py-2 text-xs bg-background h-9"
                data-testid="select-recovery-reason"
              >
                <option value="hardware_migration">Hardware Upgrade / Server Migration</option>
                <option value="corrupted_state">Corrupted License Cache / DB Restore</option>
                <option value="false_positive">False-Positive Tampering or Integrity Alert</option>
                <option value="reissue_key">Key Compromise / Reissue Required</option>
              </select>
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("license.contactEmailLabel", "Authorized Administrator Email")}
              </label>
              <Input
                type="email"
                placeholder="admin@enterprise-cloud.net"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                data-testid="input-recovery-email"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("license.recoveryNotesLabel", "Incident Notes & Context")}
              </label>
              <textarea
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                rows={3}
                placeholder="Provide server IP change, maintenance context, or integrity scan details..."
                className="w-full text-xs p-2.5 border rounded-md bg-background font-mono"
                data-testid="input-recovery-notes"
              />
            </div>

            <div className="p-2.5 rounded-lg bg-muted/40 border text-[11px] text-muted-foreground flex items-center gap-2">
              <ShieldAlert className="h-4 w-4 text-primary shrink-0" />
              <span>
                {t(
                  "license.recoveryAuditNotice",
                  "A cryptographic diagnostic proof of this node is bundled with the recovery dispatch."
                )}
              </span>
            </div>

            <DialogFooter className="pt-2">
              <Button type="button" variant="outline" onClick={handleModalClose} disabled={isSubmitting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-recovery">
                {isSubmitting ? t("common.submitting", "Submitting...") : t("license.submitRecoveryBtn", "Submit Request")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
