import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
} from "@builtpanel/ui";
import { ShieldAlert, Plus, Ban, CheckCircle2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { IpRuleAction } from "./types";

interface AddIpRuleDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  domain: string;
  defaultAction?: IpRuleAction;
  /** Only turning addresses away is possible (the web server has no allow list). */
  blockOnly?: boolean;
  onConfirm: (rule: { domain: string; ip_cidr: string; action: IpRuleAction; notes: string }) => Promise<void>;
}

// Simple IP / CIDR validator
function isValidIpOrCidr(val: string): boolean {
  const trimmed = val.trim();
  if (!trimmed) return false;
  // Match IPv4 or IPv4 CIDR, IPv6 or IPv6 CIDR
  const ipv4Pattern = /^((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?\b){4}(\/(3[0-2]|[12]?\d))?$/;
  const ipv6Pattern = /^s*((([0-9A-Fa-f]{1,4}:){7}([0-9A-Fa-f]{1,4}|:))|(([0-9A-Fa-f]{1,4}:){6}(:[0-9A-Fa-f]{1,4}|((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3})|:))|(([0-9A-Fa-f]{1,4}:){5}(((:[0-9A-Fa-f]{1,4}){1,2})|:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3})|:))|(([0-9A-Fa-f]{1,4}:){4}(((:[0-9A-Fa-f]{1,4}){1,3})|((:[0-9A-Fa-f]{1,4})?:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}))|:))|(([0-9A-Fa-f]{1,4}:){3}(((:[0-9A-Fa-f]{1,4}){1,4})|((:[0-9A-Fa-f]{1,4}){0,2}:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}))|:))|(([0-9A-Fa-f]{1,4}:){2}(((:[0-9A-Fa-f]{1,4}){1,5})|((:[0-9A-Fa-f]{1,4}){0,3}:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}))|:))|(([0-9A-Fa-f]{1,4}:){1}(((:[0-9A-Fa-f]{1,4}){1,6})|((:[0-9A-Fa-f]{1,4}){0,4}:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}))|:))|(:(((:[0-9A-Fa-f]{1,4}){1,7})|((:[0-9A-Fa-f]{1,4}){0,5}:((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}))|:)))(%.+)?(\/(12[0-8]|1[01]\d|[1-9]?\d))?s*$/;
  return ipv4Pattern.test(trimmed) || ipv6Pattern.test(trimmed);
}

export function AddIpRuleDialog({
  open,
  onOpenChange,
  domain,
  defaultAction = "block",
  blockOnly = false,
  onConfirm,
}: AddIpRuleDialogProps) {
  const { t } = useTranslation();
  const [ipCidr, setIpCidr] = useState("");
  const [action, setAction] = useState<IpRuleAction>(defaultAction);
  const [notes, setNotes] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (open) {
      setIpCidr("");
      setAction(defaultAction);
      setNotes("");
    }
  }, [open, defaultAction]);

  const isValid = isValidIpOrCidr(ipCidr);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isValid) return;
    try {
      setIsSubmitting(true);
      await onConfirm({
        domain,
        ip_cidr: ipCidr.trim(),
        action,
        notes: notes.trim(),
      });
      onOpenChange(false);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit} className="space-y-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-base font-semibold">
              {action === "block" ? (
                <Ban className="h-5 w-5 text-destructive" />
              ) : (
                <CheckCircle2 className="h-5 w-5 text-emerald-500" />
              )}
              <span data-testid="add-ip-rule-title">
                {action === "block" ? t("firewall.blockIpTitle") : t("firewall.allowIpTitle")}
              </span>
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.targetDomain")}
              </label>
              <Input
                value={domain}
                disabled
                className="mt-1 font-mono text-xs bg-muted/30"
              />
            </div>

            {!blockOnly && (
            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.actionType")}
              </label>
              <div className="mt-1.5 flex gap-2">
                <Button
                  type="button"
                  variant={action === "block" ? "default" : "outline"}
                  size="sm"
                  className={`flex-1 text-xs gap-1.5 ${action === "block" ? "bg-destructive text-destructive-foreground hover:bg-destructive/90" : ""}`}
                  data-testid="btn-select-action-block"
                  onClick={() => setAction("block")}
                >
                  <Ban className="h-3.5 w-3.5" />
                  {t("firewall.actionBlock")}
                </Button>
                <Button
                  type="button"
                  variant={action === "allow" ? "default" : "outline"}
                  size="sm"
                  className={`flex-1 text-xs gap-1.5 ${action === "allow" ? "bg-emerald-600 text-white hover:bg-emerald-700" : ""}`}
                  data-testid="btn-select-action-allow"
                  onClick={() => setAction("allow")}
                >
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  {t("firewall.actionAllow")}
                </Button>
              </div>
            </div>
            )}

            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.ipOrCidrLabel")}
              </label>
              <Input
                data-testid="input-ip-cidr"
                value={ipCidr}
                onChange={(e) => setIpCidr(e.target.value)}
                placeholder="e.g. 198.51.100.42 or 203.0.113.0/24"
                className="mt-1 font-mono text-xs"
                autoComplete="off"
              />
              {ipCidr.trim() && !isValid && (
                <p className="mt-1 text-[11px] text-destructive">
                  {t("firewall.invalidIpFormat")}
                </p>
              )}
            </div>

            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.ruleNotesLabel")}
              </label>
              <Input
                data-testid="input-rule-notes"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder={t("firewall.ruleNotesPlaceholder")}
                className="mt-1 text-xs"
              />
            </div>
          </div>

          <DialogFooter className="flex justify-end gap-2 pt-2">
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
              variant={action === "block" ? "destructive" : "default"}
              disabled={!isValid || isSubmitting}
              data-testid="btn-confirm-add-ip-rule"
              className="gap-1.5"
            >
              <Plus className="h-3.5 w-3.5" />
              {action === "block" ? t("firewall.btnSubmitBlock") : t("firewall.btnSubmitAllow")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
