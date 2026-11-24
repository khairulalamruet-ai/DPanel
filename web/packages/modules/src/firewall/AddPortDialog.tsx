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
import { Network, Plus } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface AddPortDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (port: number, protocol: "tcp" | "udp", serviceName: string) => Promise<void>;
}

export function AddPortDialog({ open, onOpenChange, onConfirm }: AddPortDialogProps) {
  const { t } = useTranslation();
  const [port, setPort] = useState<number | "">("");
  const [protocol, setProtocol] = useState<"tcp" | "udp">("tcp");
  const [serviceName, setServiceName] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (open) {
      setPort("");
      setProtocol("tcp");
      setServiceName("");
    }
  }, [open]);

  const isValidPort = typeof port === "number" && port >= 1 && port <= 65535;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isValidPort) return;
    try {
      setIsSubmitting(true);
      await onConfirm(port, protocol, serviceName.trim());
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
              <Network className="h-5 w-5 text-primary" />
              <span data-testid="add-port-dialog-title">{t("firewall.openPortTitle")}</span>
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div className="grid grid-cols-3 gap-2">
              <div className="col-span-2">
                <label className="text-xs font-medium text-foreground">
                  {t("firewall.portNumber")} (1-65535)
                </label>
                <Input
                  data-testid="input-port-number"
                  type="number"
                  min={1}
                  max={65535}
                  value={port}
                  onChange={(e) => setPort(e.target.value === "" ? "" : Number(e.target.value))}
                  placeholder="e.g. 8080"
                  className="mt-1 font-mono text-xs"
                />
              </div>
              <div>
                <label className="text-xs font-medium text-foreground">
                  {t("firewall.protocol")}
                </label>
                <select
                  data-testid="select-port-protocol"
                  value={protocol}
                  onChange={(e) => setProtocol(e.target.value as "tcp" | "udp")}
                  className="mt-1 w-full rounded-md border border-input bg-background px-2.5 py-1.5 text-xs font-mono h-9 shadow-sm focus:outline-none"
                >
                  <option value="tcp">TCP</option>
                  <option value="udp">UDP</option>
                </select>
              </div>
            </div>

            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.serviceDescription")}
              </label>
              <Input
                data-testid="input-port-service"
                value={serviceName}
                onChange={(e) => setServiceName(e.target.value)}
                placeholder="e.g. Custom Webhook Service"
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
              variant="default"
              disabled={!isValidPort || isSubmitting}
              data-testid="btn-confirm-open-port"
              className="gap-1.5"
            >
              <Plus className="h-3.5 w-3.5" />
              {t("firewall.btnOpenPort")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
