import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
} from "@builtpanel/ui";
import { installCustomCertificate } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import { UploadCloud, AlertCircle, Key, FileCheck } from "lucide-react";
import type { SslCertificateItem } from "./types";

interface CustomCertUploadDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  domainId: string;
  domainName: string;
  onSuccess: (updatedCert: SslCertificateItem) => void;
}

export function CustomCertUploadDialog({
  open,
  onOpenChange,
  domainId,
  domainName,
  onSuccess,
}: CustomCertUploadDialogProps) {
  const { t } = useTranslation();

  const [certPem, setCertPem] = useState("");
  const [keyPem, setKeyPem] = useState("");
  const [chainPem, setChainPem] = useState("");

  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleReset = () => {
    setCertPem("");
    setKeyPem("");
    setChainPem("");
    setError(null);
  };

  const handleClose = () => {
    handleReset();
    onOpenChange(false);
  };

  const validate = (): string | null => {
    if (!certPem.trim()) {
      return t("ssl.errors.certRequired", "Certificate PEM content is required");
    }
    if (!certPem.includes("BEGIN CERTIFICATE") || !certPem.includes("END CERTIFICATE")) {
      return t(
        "ssl.errors.invalidCertPem",
        "Invalid Certificate PEM: must include '-----BEGIN CERTIFICATE-----' and '-----END CERTIFICATE-----'"
      );
    }

    if (!keyPem.trim()) {
      return t("ssl.errors.keyRequired", "Private Key PEM content is required");
    }
    if (!keyPem.includes("BEGIN PRIVATE KEY") && !keyPem.includes("BEGIN RSA PRIVATE KEY")) {
      return t(
        "ssl.errors.invalidKeyPem",
        "Invalid Private Key PEM: must include '-----BEGIN PRIVATE KEY-----' or '-----BEGIN RSA PRIVATE KEY-----'"
      );
    }

    return null;
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    const validationError = validate();
    if (validationError) {
      setError(validationError);
      return;
    }

    try {
      setIsSubmitting(true);
      const res = await installCustomCertificate(domainId, {
        certificate: certPem,
        chain: chainPem,
        privateKey: keyPem,
      });

      onSuccess(res);
      handleClose();
    } catch (err: any) {
      setError(err?.message || t("ssl.errors.uploadFailed", "Failed to install custom SSL certificate"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-xl" data-testid="dialog-custom-ssl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <UploadCloud className="h-5 w-5 text-primary" />
            {t("ssl.customDialog.title", "Install Custom SSL Certificate")}
          </DialogTitle>
        </DialogHeader>

        <p className="text-xs text-muted-foreground">
          {t(
            "ssl.customDialog.description",
            "Upload an existing commercial or enterprise PEM certificate and private key for {{domain}}.",
            { domain: domainName }
          )}
        </p>

        {error && (
          <div className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4 py-2 text-sm">
          {/* Certificate PEM */}
          <div className="space-y-1.5">
            <label htmlFor="ssl-cert-pem" className="text-xs font-semibold flex items-center gap-1.5">
              <FileCheck className="h-3.5 w-3.5 text-primary" />
              {t("ssl.fields.certPem", "Certificate (CRT / PEM)")}
            </label>
            <textarea
              id="ssl-cert-pem"
              rows={4}
              value={certPem}
              onChange={(e) => setCertPem(e.target.value)}
              placeholder="-----BEGIN CERTIFICATE-----&#10;MIID...&#10;-----END CERTIFICATE-----"
              className="w-full p-2.5 rounded-md border border-input bg-background font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              data-testid="textarea-ssl-cert-pem"
            />
          </div>

          {/* Private Key PEM */}
          <div className="space-y-1.5">
            <label htmlFor="ssl-key-pem" className="text-xs font-semibold flex items-center gap-1.5">
              <Key className="h-3.5 w-3.5 text-amber-500" />
              {t("ssl.fields.keyPem", "Private Key (KEY / PEM)")}
            </label>
            <textarea
              id="ssl-key-pem"
              rows={4}
              value={keyPem}
              onChange={(e) => setKeyPem(e.target.value)}
              placeholder="-----BEGIN PRIVATE KEY-----&#10;MIIE...&#10;-----END PRIVATE KEY-----"
              className="w-full p-2.5 rounded-md border border-input bg-background font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              data-testid="textarea-ssl-key-pem"
            />
          </div>

          {/* CA Chain (Optional) */}
          <div className="space-y-1.5">
            <label htmlFor="ssl-chain-pem" className="text-xs font-semibold text-muted-foreground">
              {t("ssl.fields.chainPem", "CA Bundle / Intermediate Certificates (Optional)")}
            </label>
            <textarea
              id="ssl-chain-pem"
              rows={3}
              value={chainPem}
              onChange={(e) => setChainPem(e.target.value)}
              placeholder="-----BEGIN CERTIFICATE-----&#10;Intermediate CA...&#10;-----END CERTIFICATE-----"
              className="w-full p-2.5 rounded-md border border-input bg-background font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              data-testid="textarea-ssl-chain-pem"
            />
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={handleClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-custom-ssl">
              <UploadCloud className="h-4 w-4 mr-1.5" />
              {isSubmitting ? t("common.processing", "Verifying...") : t("ssl.actions.installCert", "Install Certificate")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
