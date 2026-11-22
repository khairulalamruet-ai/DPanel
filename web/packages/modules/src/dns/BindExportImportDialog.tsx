import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
} from "@builtpanel/ui";
import { toBind, importBind } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import { FileText, Download, Copy, Check, UploadCloud, AlertCircle } from "lucide-react";
import type { BindImportResponse, DnsRecord } from "./types";

interface BindExportImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  zoneId: string;
  domainName: string;
  /** The records the zone has now (what an export writes). */
  records: DnsRecord[];
  nameservers: string[];
  onImportSuccess: () => void;
}

export function BindExportImportDialog({
  open,
  onOpenChange,
  zoneId,
  domainName,
  records,
  nameservers,
  onImportSuccess,
}: BindExportImportDialogProps) {
  const { t } = useTranslation();

  const [activeTab, setActiveTab] = useState<"export" | "import">("export");

  // Export state
  const [bindExportText, setBindExportText] = useState("");
  const [isExportLoading, setIsExportLoading] = useState(false);
  const [hasCopied, setHasCopied] = useState(false);

  // Import state
  const [bindImportText, setBindImportText] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [importResult, setImportResult] = useState<BindImportResponse | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  useEffect(() => {
    if (open && activeTab === "export" && zoneId) {
      fetchExport();
    }
  }, [open, activeTab, zoneId]);

  const fetchExport = async () => {
    try {
      setIsExportLoading(true);
      setBindExportText(toBind(domainName, records, nameservers));
    } catch {
      setBindExportText(`; Error exporting zone ${domainName}`);
    } finally {
      setIsExportLoading(false);
    }
  };

  const handleCopy = async () => {
    if (!bindExportText) return;
    await navigator.clipboard.writeText(bindExportText);
    setHasCopied(true);
    setTimeout(() => setHasCopied(false), 2000);
  };

  const handleDownload = () => {
    const element = document.createElement("a");
    const file = new Blob([bindExportText], { type: "text/plain" });
    element.href = URL.createObjectURL(file);
    element.download = `${domainName}.zone`;
    document.body.appendChild(element);
    element.click();
    document.body.removeChild(element);
  };

  const handleImportSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setImportError(null);
    setImportResult(null);

    if (!bindImportText.trim()) {
      setImportError(
        t("dns.bind.emptyError", "Please paste valid BIND zone file contents.")
      );
      return;
    }

    try {
      setIsImporting(true);
      const res = await importBind(zoneId, domainName, bindImportText, overwrite);

      setImportResult(res);
      onImportSuccess();
    } catch (err: any) {
      setImportError(err?.message || t("dns.bind.importError", "Failed to import BIND zone file"));
    } finally {
      setIsImporting(false);
    }
  };

  const handleClose = () => {
    setImportResult(null);
    setImportError(null);
    setBindImportText("");
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileText className="h-5 w-5 text-primary" />
            {t("dns.bind.dialogTitle", "BIND Zone File Management")}
          </DialogTitle>
        </DialogHeader>

        {/* Tab switchers */}
        <div className="flex border-b text-xs font-medium">
          <button
            type="button"
            className={`px-4 py-2 border-b-2 font-semibold transition-colors ${
              activeTab === "export"
                ? "border-primary text-primary"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
            onClick={() => setActiveTab("export")}
            data-testid="tab-bind-export"
          >
            {t("dns.bind.exportTab", "Export BIND Zone")}
          </button>
          <button
            type="button"
            className={`px-4 py-2 border-b-2 font-semibold transition-colors ${
              activeTab === "import"
                ? "border-primary text-primary"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
            onClick={() => setActiveTab("import")}
            data-testid="tab-bind-import"
          >
            {t("dns.bind.importTab", "Import BIND Zone")}
          </button>
        </div>

        {activeTab === "export" ? (
          /* Export Tab Content */
          <div className="space-y-4 py-2">
            <p className="text-xs text-muted-foreground">
              {t(
                "dns.bind.exportDesc",
                "Standard RFC 1035 BIND zone file for {{domain}}. Compatible with PowerDNS, BIND9, and Cloudflare DNS imports.",
                { domain: domainName }
              )}
            </p>

            <div className="relative">
              <pre
                className="w-full h-64 p-3 bg-muted font-mono text-xs rounded-md border overflow-auto select-all"
                data-testid="pre-bind-export"
              >
                {isExportLoading ? t("common.loading", "Loading zone...") : bindExportText}
              </pre>
            </div>

            <DialogFooter className="flex justify-between sm:justify-between items-center pt-2">
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="text-xs gap-1.5"
                  onClick={handleCopy}
                  data-testid="btn-copy-bind"
                >
                  {hasCopied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                  {hasCopied ? t("common.copied", "Copied!") : t("common.copy", "Copy Text")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="text-xs gap-1.5"
                  onClick={handleDownload}
                  data-testid="btn-download-bind"
                >
                  <Download className="h-3.5 w-3.5" />
                  {t("dns.bind.downloadZone", "Download .zone")}
                </Button>
              </div>
              <Button type="button" onClick={handleClose}>
                {t("common.close", "Close")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          /* Import Tab Content */
          <form onSubmit={handleImportSubmit} className="space-y-4 py-2 text-sm">
            <p className="text-xs text-muted-foreground">
              {t(
                "dns.bind.importDesc",
                "Paste BIND-formatted zone file records to batch import or restore your DNS zone."
              )}
            </p>

            {importError && (
              <div className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg flex items-center gap-2">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{importError}</span>
              </div>
            )}

            {importResult && (
              <div
                className="p-3 text-xs bg-emerald-500/10 border border-emerald-500/20 text-emerald-800 dark:text-emerald-300 rounded-lg"
                data-testid="bind-import-success-notice"
              >
                <p className="font-semibold">
                  {t("dns.bind.importSuccess", "Successfully imported {{count}} DNS records into zone.", {
                    count: importResult.imported_count,
                  })}
                </p>
              </div>
            )}

            <div className="space-y-1.5">
              <label htmlFor="bind-import-text" className="text-xs font-semibold">
                {t("dns.bind.pasteLabel", "BIND Zone Content")}
              </label>
              <textarea
                id="bind-import-text"
                rows={8}
                value={bindImportText}
                onChange={(e) => setBindImportText(e.target.value)}
                placeholder={`; Paste zone lines, for example:\n@ IN A 192.168.1.100\nwww IN CNAME alpha-hosting.com.\n@ IN MX 10 mail.alpha-hosting.com.`}
                className="w-full p-2.5 rounded-md border border-input bg-background font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                data-testid="textarea-bind-import"
              />
            </div>

            <div className="flex items-center gap-2">
              <input
                type="checkbox"
                id="bind-overwrite"
                checked={overwrite}
                onChange={(e) => setOverwrite(e.target.checked)}
                className="rounded border-input text-primary focus:ring-primary h-4 w-4"
                data-testid="checkbox-bind-overwrite"
              />
              <label htmlFor="bind-overwrite" className="cursor-pointer text-xs font-medium">
                {t("dns.bind.overwriteLabel", "Overwrite existing records in this zone")}
              </label>
            </div>

            <DialogFooter className="pt-2">
              <Button type="button" variant="outline" onClick={handleClose} disabled={isImporting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={isImporting} data-testid="btn-submit-bind-import">
                <UploadCloud className="h-4 w-4 mr-1.5" />
                {isImporting ? t("common.processing", "Importing...") : t("dns.bind.importBtn", "Import Records")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
