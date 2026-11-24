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
import { createRecord } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import { Plus, AlertCircle } from "lucide-react";
import type { DnsRecord, DnsRecordType, CreateDnsRecordPayload } from "./types";

interface DnsRecordFormDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  zoneId: string;
  domainName: string;
  onSuccess: (newRecord: DnsRecord) => void;
}

const RECORD_TYPES: DnsRecordType[] = [
  "A",
  "AAAA",
  "CNAME",
  "MX",
  "TXT",
  "SRV",
  "CAA",
  "NS",
];

const COMMON_TTLS = [
  { value: 60, label: "1 min (60s)" },
  { value: 300, label: "5 mins (300s)" },
  { value: 3600, label: "1 hour (3600s)" },
  { value: 14400, label: "4 hours (14400s)" },
  { value: 86400, label: "1 day (86400s)" },
];

export function DnsRecordFormDialog({
  open,
  onOpenChange,
  zoneId,
  domainName,
  onSuccess,
}: DnsRecordFormDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("@");
  const [type, setType] = useState<DnsRecordType>("A");
  const [content, setContent] = useState("");
  const [ttl, setTtl] = useState(3600);
  const [priority, setPriority] = useState<number | "">("");
  const [weight, setWeight] = useState<number | "">("");
  const [port, setPort] = useState<number | "">("");
  const [flags, setFlags] = useState<number | "">(0);
  const [tag, setTag] = useState("issue");

  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleReset = () => {
    setName("@");
    setType("A");
    setContent("");
    setTtl(3600);
    setPriority("");
    setWeight("");
    setPort("");
    setFlags(0);
    setTag("issue");
    setError(null);
  };

  const handleClose = () => {
    handleReset();
    onOpenChange(false);
  };

  const validate = (): string | null => {
    if (!name.trim()) {
      return t("dns.errors.nameRequired", "Record name is required (use @ for root)");
    }
    if (!content.trim()) {
      return t("dns.errors.contentRequired", "Record content value is required");
    }

    if (type === "A") {
      const ipv4Regex =
        /^(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$/;
      if (!ipv4Regex.test(content.trim())) {
        return t("dns.errors.invalidIpv4", "Please enter a valid IPv4 address (e.g. 192.168.1.1)");
      }
    }

    if (type === "AAAA") {
      if (!content.includes(":")) {
        return t("dns.errors.invalidIpv6", "Please enter a valid IPv6 address (e.g. 2001:db8::1)");
      }
    }

    if (type === "MX" && (priority === "" || Number(priority) < 0 || Number(priority) > 65535)) {
      return t("dns.errors.invalidPriority", "Valid MX priority between 0 and 65535 is required");
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
      const payload: CreateDnsRecordPayload = {
        name: name.trim(),
        type,
        content: content.trim(),
        ttl: Number(ttl),
        priority: type === "MX" || type === "SRV" ? (priority !== "" ? Number(priority) : null) : null,
        weight: type === "SRV" ? (weight !== "" ? Number(weight) : null) : null,
        port: type === "SRV" ? (port !== "" ? Number(port) : null) : null,
        flags: type === "CAA" ? (flags !== "" ? Number(flags) : null) : null,
        tag: type === "CAA" ? tag : null,
      };

      const res = await createRecord(zoneId, payload);

      onSuccess(res);
      handleClose();
    } catch (err: any) {
      setError(err?.message || t("dns.errors.createFailed", "Failed to create DNS record"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Plus className="h-5 w-5 text-primary" />
            {t("dns.createDialog.title", "Add DNS Record")}
          </DialogTitle>
        </DialogHeader>

        {error && (
          <div className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4 py-2 text-sm">
          {/* Type & Name */}
          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1.5 col-span-1">
              <label htmlFor="dns-type" className="text-xs font-semibold">
                {t("dns.fields.type", "Type")}
              </label>
              <select
                id="dns-type"
                value={type}
                onChange={(e) => setType(e.target.value as DnsRecordType)}
                className="w-full h-9 rounded-md border border-input bg-background px-3 py-1 font-semibold text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                data-testid="select-record-type"
              >
                {RECORD_TYPES.map((rt) => (
                  <option key={rt} value={rt}>
                    {rt}
                  </option>
                ))}
              </select>
            </div>

            <div className="space-y-1.5 col-span-2">
              <label htmlFor="dns-name" className="text-xs font-semibold">
                {t("dns.fields.name", "Host / Name")}
              </label>
              <div className="flex items-center">
                <Input
                  id="dns-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="@ or subdomain"
                  className="h-9 text-xs font-mono rounded-r-none"
                  data-testid="input-record-name"
                />
                <span className="h-9 inline-flex items-center px-2 bg-muted border border-l-0 border-input rounded-r-md text-[11px] text-muted-foreground font-mono truncate max-w-[120px]">
                  .{domainName}
                </span>
              </div>
            </div>
          </div>

          {/* Content */}
          <div className="space-y-1.5">
            <label htmlFor="dns-content" className="text-xs font-semibold">
              {t("dns.fields.content", "Target / Value")}
            </label>
            <Input
              id="dns-content"
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={
                type === "A"
                  ? "192.168.1.100"
                  : type === "AAAA"
                  ? "2001:db8::1"
                  : type === "CNAME"
                  ? "target-domain.com."
                  : type === "MX"
                  ? "mail.example.com."
                  : type === "TXT"
                  ? "v=spf1 a mx ~all"
                  : "Content target"
              }
              className="h-9 text-xs font-mono"
              data-testid="input-record-content"
            />
          </div>

          {/* Type-Specific fields */}
          {type === "MX" && (
            <div className="space-y-1.5">
              <label htmlFor="dns-priority" className="text-xs font-semibold">
                {t("dns.fields.priority", "MX Priority (0 - 65535)")}
              </label>
              <Input
                id="dns-priority"
                type="number"
                min="0"
                max="65535"
                value={priority}
                onChange={(e) => setPriority(e.target.value === "" ? "" : Number(e.target.value))}
                placeholder="10"
                className="h-9 text-xs font-mono"
                data-testid="input-record-priority"
              />
            </div>
          )}

          {type === "SRV" && (
            <div className="grid grid-cols-3 gap-2">
              <div className="space-y-1">
                <label className="text-xs">{t("dns.fields.priority", "Priority")}</label>
                <Input
                  type="number"
                  value={priority}
                  onChange={(e) => setPriority(e.target.value === "" ? "" : Number(e.target.value))}
                  placeholder="10"
                  className="h-8 text-xs font-mono"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs">{t("dns.fields.weight", "Weight")}</label>
                <Input
                  type="number"
                  value={weight}
                  onChange={(e) => setWeight(e.target.value === "" ? "" : Number(e.target.value))}
                  placeholder="5"
                  className="h-8 text-xs font-mono"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs">{t("dns.fields.port", "Port")}</label>
                <Input
                  type="number"
                  value={port}
                  onChange={(e) => setPort(e.target.value === "" ? "" : Number(e.target.value))}
                  placeholder="5060"
                  className="h-8 text-xs font-mono"
                />
              </div>
            </div>
          )}

          {type === "CAA" && (
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <label className="text-xs">{t("dns.fields.flags", "Flags")}</label>
                <Input
                  type="number"
                  value={flags}
                  onChange={(e) => setFlags(e.target.value === "" ? "" : Number(e.target.value))}
                  className="h-9 text-xs font-mono"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs">{t("dns.fields.tag", "Tag")}</label>
                <select
                  value={tag}
                  onChange={(e) => setTag(e.target.value)}
                  className="w-full h-9 rounded-md border border-input bg-background px-3 py-1 text-xs"
                >
                  <option value="issue">issue</option>
                  <option value="issuewild">issuewild</option>
                  <option value="iodef">iodef</option>
                </select>
              </div>
            </div>
          )}

          {/* TTL */}
          <div className="space-y-1.5">
            <label htmlFor="dns-ttl" className="text-xs font-semibold">
              {t("dns.fields.ttl", "TTL (Time to Live)")}
            </label>
            <select
              id="dns-ttl"
              value={ttl}
              onChange={(e) => setTtl(Number(e.target.value))}
              className="w-full h-9 rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              data-testid="select-record-ttl"
            >
              {COMMON_TTLS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={handleClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-dns-record">
              {isSubmitting ? t("common.processing", "Saving...") : t("dns.actions.addRecord", "Add Record")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
