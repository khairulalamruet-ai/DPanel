import React, { useState, useEffect, useCallback } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
  Input,
  useToast,
} from "@builtpanel/ui";
import { getPreview, createPreview, deletePreview } from "./api";
import {
  Globe2,
  Copy,
  Check,
  ExternalLink,
  Clock,
  AlertTriangle,
  RefreshCw,
  Trash2,
  ShieldCheck,
} from "lucide-react";
import { ClientDomainItem, PreviewLinkInfo } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface PreviewLinkDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  domain: ClientDomainItem | null;
}

export function PreviewLinkDialog({
  open,
  onOpenChange,
  domain,
}: PreviewLinkDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [previewInfo, setPreviewInfo] = useState<PreviewLinkInfo | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isGenerating, setIsGenerating] = useState(false);
  const [isRevoking, setIsRevoking] = useState(false);
  const [copied, setCopied] = useState(false);

  // Fetch status when dialog opens
  const fetchPreviewStatus = useCallback(async () => {
    if (!domain) return;
    try {
      setIsLoading(true);
      const res = await getPreview(domain);
      setPreviewInfo(res);
    } catch {
      // Default to empty state on 404 or first load
      setPreviewInfo({
        enabled: false,
        domain_id: domain.id,
        target_domain: domain.name,
      });
    } finally {
      setIsLoading(false);
    }
  }, [domain]);

  useEffect(() => {
    if (open && domain) {
      fetchPreviewStatus();
      setCopied(false);
    }
  }, [open, domain, fetchPreviewStatus]);

  const handleGenerate = async () => {
    if (!domain) return;
    try {
      setIsGenerating(true);
      const res = await createPreview(domain);
      setPreviewInfo(res);
      toast({
        title: t("common.success"),
        description: t("domains.preview.toastGenerated", "Temporary preview link generated successfully"),
        variant: "default",
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("domains.preview.errorGenerate", "Failed to generate preview link"),
        variant: "destructive",
      });
    } finally {
      setIsGenerating(false);
    }
  };

  const handleRevoke = async () => {
    if (!domain) return;
    try {
      setIsRevoking(true);
      await deletePreview(domain);
      setPreviewInfo({
        enabled: false,
        domain_id: domain.id,
        target_domain: domain.name,
      });
      toast({
        title: t("common.success"),
        description: t("domains.preview.toastRevoked", "Preview link revoked"),
        variant: "default",
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("domains.preview.errorRevoke", "Failed to revoke preview link"),
        variant: "destructive",
      });
    } finally {
      setIsRevoking(false);
    }
  };

  const handleCopy = () => {
    if (!previewInfo?.url) return;
    navigator.clipboard?.writeText(previewInfo.url);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
    toast({
      title: t("common.copied", "Copied to clipboard"),
      description: previewInfo.url,
      variant: "default",
    });
  };

  if (!domain) return null;

  const isActive = previewInfo?.enabled && !previewInfo.is_expired;
  const isExpired = previewInfo?.enabled && previewInfo.is_expired;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Globe2 className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("domains.preview.dialogTitle", "Temporary Preview Link")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs">
                {domain.name}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="mt-4 space-y-4">
          {isLoading ? (
            <div className="flex h-32 items-center justify-center">
              <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
            </div>
          ) : isActive && previewInfo?.url ? (
            /* Active Preview Link Card */
            <div className="space-y-4">
              <div className="flex items-center justify-between border-b border-border pb-3">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-semibold text-foreground">
                    {t("domains.preview.status", "Status")}:
                  </span>
                  <Badge variant="success" className="text-xs" data-testid="badge-preview-status-active">
                    {t("domains.preview.active", "Active")}
                  </Badge>
                </div>
                {previewInfo.expires_at && (
                  <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Clock className="h-3.5 w-3.5" />
                    <span>
                      {t("domains.preview.expires", "Expires")}: {new Date(previewInfo.expires_at).toLocaleDateString()}
                    </span>
                  </div>
                )}
              </div>

              {/* Link Input & Copy/Open actions */}
              <div className="space-y-2">
                <label className="text-xs font-medium text-foreground">
                  {t("domains.preview.previewUrl", "Preview URL")}
                </label>
                <div className="flex items-center gap-2">
                  <Input
                    readOnly
                    value={previewInfo.url}
                    className="h-9 font-mono text-xs text-foreground bg-muted/40"
                    data-testid="input-preview-url"
                  />
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={handleCopy}
                    title={t("common.copy", "Copy URL")}
                    data-testid="btn-copy-preview-url"
                  >
                    {copied ? <Check className="h-4 w-4 text-emerald-500" /> : <Copy className="h-4 w-4" />}
                  </Button>
                  <a
                    href={previewInfo.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center justify-center rounded-md text-xs font-medium border border-input bg-background hover:bg-accent hover:text-accent-foreground h-9 px-3"
                    data-testid="btn-open-preview-url"
                  >
                    <ExternalLink className="h-4 w-4" />
                  </a>
                </div>
              </div>

              {/* Explanation note */}
              <div className="rounded-lg border border-border bg-muted/20 p-3 text-xs text-muted-foreground">
                <div className="flex items-center gap-1.5 font-medium text-foreground mb-1">
                  <ShieldCheck className="h-3.5 w-3.5 text-emerald-500" />
                  <span>{t("domains.preview.directRouting", "Direct Container Proxy")}</span>
                </div>
                <p>
                  {t(
                    "domains.preview.routingNote",
                    "Requests to this URL proxy directly to your web application container without requiring external DNS configuration."
                  )}
                </p>
              </div>

              {/* Actions: Revoke or Extend */}
              <div className="flex items-center justify-between pt-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleRevoke}
                  disabled={isRevoking}
                  className="text-xs text-destructive hover:bg-destructive/10"
                  data-testid="btn-revoke-preview-link"
                >
                  <Trash2 className="h-3.5 w-3.5 mr-1" />
                  {isRevoking ? t("common.loading") : t("domains.preview.revoke", "Revoke Link")}
                </Button>

                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    onClick={handleGenerate}
                    disabled={isGenerating}
                    className="text-xs"
                    data-testid="btn-extend-preview-link"
                  >
                    <RefreshCw className={`h-3.5 w-3.5 mr-1 ${isGenerating ? "animate-spin" : ""}`} />
                    {isGenerating ? t("common.loading") : t("domains.preview.extend", "Make a new link")}
                  </Button>
                </div>
              </div>
            </div>
          ) : (
            /* Expired or Not Enabled State */
            <div className="space-y-4">
              {isExpired ? (
                <div className="rounded-lg border border-destructive/20 bg-destructive/10 p-3.5 text-xs text-destructive" data-testid="banner-preview-expired">
                  <div className="flex items-center gap-2 font-semibold mb-1">
                    <AlertTriangle className="h-4 w-4" />
                    <span>{t("domains.preview.expiredTitle", "Preview Link Expired")}</span>
                  </div>
                  <p>
                    {t(
                      "domains.preview.expiredDesc",
                      "The previous temporary link has elapsed its validity period and requests are now rejected. Generate a new link below to resume previewing."
                    )}
                  </p>
                </div>
              ) : (
                <div className="rounded-lg border border-border bg-muted/30 p-3.5 text-xs text-muted-foreground">
                  <p>
                    {t(
                      "domains.preview.intro",
                      "Preview your site immediately before DNS records propagate worldwide. A temporary URL will be created and expires on its own."
                    )}
                  </p>
                </div>
              )}

              <div className="pt-2">
                <Button
                  onClick={handleGenerate}
                  disabled={isGenerating}
                  className="w-full text-xs"
                  data-testid="btn-generate-preview-link"
                >
                  <Globe2 className="h-4 w-4 mr-2" />
                  {isGenerating
                    ? t("common.generating", "Generating...")
                    : t("domains.preview.generateBtn", "Generate Temporary Preview Link")}
                </Button>
              </div>
            </div>
          )}
        </div>

        <DialogFooter className="mt-6 border-t border-border pt-3">
          <Button
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            data-testid="btn-close-preview-dialog"
          >
            {t("common.close", "Close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
