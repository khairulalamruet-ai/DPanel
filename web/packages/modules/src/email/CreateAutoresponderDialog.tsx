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
import { Clock } from "lucide-react";

interface CreateAutoresponderDialogProps {
  open: boolean;
  domain: string;
  onClose: () => void;
  onCreate: (data: {
    email: string;
    domain: string;
    subject: string;
    body: string;
    start_date?: string;
    end_date?: string;
  }) => Promise<void>;
}

export function CreateAutoresponderDialog({
  open,
  domain,
  onClose,
  onCreate,
}: CreateAutoresponderDialogProps) {
  const { t } = useTranslation();

  const [emailUser, setEmailUser] = useState("");
  const [subject, setSubject] = useState("Out of Office: Auto-Response");
  const [body, setBody] = useState("Thank you for reaching out. I am currently out of the office and will reply as soon as possible upon my return.");
  const [startDate, setStartDate] = useState("");
  const [endDate, setEndDate] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    const cleanUser = emailUser.trim().toLowerCase().replace(/@.*$/, "");
    if (!cleanUser) {
      setErrorMsg("Please enter an email username");
      return;
    }
    if (!subject.trim()) {
      setErrorMsg("Please enter an email subject line");
      return;
    }
    if (!body.trim()) {
      setErrorMsg("Please enter an auto-reply body message");
      return;
    }

    setIsSubmitting(true);
    try {
      await onCreate({
        email: `${cleanUser}@${domain}`,
        domain,
        subject: subject.trim(),
        body: body.trim(),
        start_date: startDate ? new Date(startDate).toISOString() : undefined,
        end_date: endDate ? new Date(endDate).toISOString() : undefined,
      });
      setEmailUser("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to create autoresponder");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="create-autoresponder-dialog" className="sm:max-w-lg">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Clock className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-autoresponder-title">
                {t("email.createAutoresponderTitle", "Set Up Autoresponder")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("email.createAutoresponderSubtitle", "Configure automatic replies for vacation or away notices.")}
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
              {t("email.autoresponderEmailLabel", "Email Address")}
            </label>
            <div className="flex items-center gap-2">
              <Input
                value={emailUser}
                onChange={(e) => setEmailUser(e.target.value)}
                placeholder="support"
                required
                className="flex-1"
                data-testid="autoresponder-email-input"
              />
              <span className="text-sm font-mono text-muted-foreground">@{domain}</span>
            </div>
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("email.autoresponderSubjectLabel", "Subject")}
            </label>
            <Input
              value={subject}
              onChange={(e) => setSubject(e.target.value)}
              placeholder="Out of Office"
              required
              data-testid="autoresponder-subject-input"
            />
          </div>

          <div>
            <label className="text-xs font-semibold block mb-1">
              {t("email.autoresponderBodyLabel", "Message Body")}
            </label>
            <textarea
              value={body}
              onChange={(e) => setBody(e.target.value)}
              rows={4}
              required
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              placeholder="I am currently unavailable..."
              data-testid="autoresponder-body-input"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("email.autoresponderStartLabel", "Start Date (Optional)")}
              </label>
              <Input
                type="datetime-local"
                value={startDate}
                onChange={(e) => setStartDate(e.target.value)}
                className="text-xs"
              />
            </div>
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("email.autoresponderEndLabel", "End Date (Optional)")}
              </label>
              <Input
                type="datetime-local"
                value={endDate}
                onChange={(e) => setEndDate(e.target.value)}
                className="text-xs"
              />
            </div>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="submit-autoresponder-btn">
              {isSubmitting ? t("common.saving", "Saving...") : t("email.saveAutoresponderBtn", "Save Autoresponder")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
