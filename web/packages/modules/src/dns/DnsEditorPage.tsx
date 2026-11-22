import React, { useState, useEffect, useMemo, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Input,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  useToast,
} from "@builtpanel/ui";
import { listZones, loadZone, updateRecord, deleteRecord, verifyNameservers } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Globe,
  Plus,
  FileText,
  Search,
  RefreshCw,
  ShieldCheck,
  Server,
  Layers,
} from "lucide-react";
import type {
  DnsZone,
  DnsRecord,
  DnsRecordType,
  DnsZoneDetailResponse,
  UpdateDnsRecordPayload,
} from "./types";
import { DnsRecordFormDialog } from "./DnsRecordFormDialog";
import { InlineRecordRow } from "./InlineRecordRow";
import { BindExportImportDialog } from "./BindExportImportDialog";

const FILTER_TYPES: Array<DnsRecordType | "ALL"> = [
  "ALL",
  "A",
  "AAAA",
  "CNAME",
  "MX",
  "TXT",
  "SRV",
  "CAA",
];

export function DnsEditorPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [zones, setZones] = useState<DnsZone[]>([]);
  const [selectedZoneId, setSelectedZoneId] = useState<string>("");
  const [activeZone, setActiveZone] = useState<DnsZone | null>(null);
  const [records, setRecords] = useState<DnsRecord[]>([]);

  const [isLoading, setIsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedType, setSelectedType] = useState<DnsRecordType | "ALL">("ALL");

  // Inline edit state
  const [editingRecordId, setEditingRecordId] = useState<string | null>(null);

  // Dialogs
  const [isAddOpen, setIsAddOpen] = useState(false);
  const [isBindOpen, setIsBindOpen] = useState(false);
  const [deletingRecord, setDeletingRecord] = useState<DnsRecord | null>(null);

  // NS Verification state
  const [isVerifyingNs, setIsVerifyingNs] = useState(false);

  // 1. Fetch all zones
  const fetchZones = useCallback(async () => {
    try {
      setIsLoading(true);
      const items = await listZones();
      setZones(items);
      if (items.length > 0) {
        setSelectedZoneId((current) => current || items[0].id);
      }
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("dns.errors.fetchZonesFailed", "Failed to load DNS zones"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchZones();
  }, [fetchZones]);

  // 2. Fetch specific zone records when selectedZoneId changes
  const fetchZoneDetails = useCallback(async (zoneId: string) => {
    if (!zoneId) return;
    try {
      setIsLoading(true);
      const base = zones.find((z) => z.id === zoneId);
      if (!base) return;
      const res = await loadZone(base);
      setActiveZone(res.zone);
      setRecords(res.records);
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("dns.errors.fetchDetailsFailed", "Failed to load zone records"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [zones, t, toast]);

  useEffect(() => {
    if (selectedZoneId && zones.length > 0) {
      fetchZoneDetails(selectedZoneId);
    }
  }, [selectedZoneId, fetchZoneDetails]);

  // 3. Filtered records
  const filteredRecords = useMemo(() => {
    let result = records;

    if (selectedType !== "ALL") {
      result = result.filter((r) => r.type === selectedType);
    }

    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase().trim();
      result = result.filter(
        (r) =>
          r.name.toLowerCase().includes(q) ||
          r.content.toLowerCase().includes(q) ||
          r.type.toLowerCase().includes(q)
      );
    }

    return result;
  }, [records, selectedType, searchQuery]);

  // 4. Save inline edited record
  const handleSaveInline = async (recordId: string, payload: UpdateDnsRecordPayload) => {
    try {
      const current = records.find((r) => r.id === recordId);
      if (!current || !selectedZoneId) return;
      const updated = await updateRecord(selectedZoneId, current, payload);

      setRecords((prev) => prev.map((r) => (r.id === recordId ? updated : r)));
      setEditingRecordId(null);
      toast({
        title: t("common.success", "Success"),
        description: t("dns.toast.recordUpdated", "DNS record updated successfully"),
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("dns.errors.updateFailed", "Failed to update record"),
        variant: "destructive",
      });
    }
  };

  // 5. Delete record
  const handleDeleteRecord = async () => {
    if (!deletingRecord) return;
    try {
      await deleteRecord(selectedZoneId as string, deletingRecord.id);
      setRecords((prev) => prev.filter((r) => r.id !== deletingRecord.id));
      toast({
        title: t("common.success", "Success"),
        description: t("dns.toast.recordDeleted", "DNS record deleted successfully"),
      });
      setDeletingRecord(null);
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("dns.errors.deleteFailed", "Failed to delete record"),
        variant: "destructive",
      });
    }
  };

  // 6. Verify NS
  const handleVerifyNs = async () => {
    if (!selectedZoneId) return;
    try {
      setIsVerifyingNs(true);
      const res = await verifyNameservers(selectedZoneId);
      toast({
        title: res.ok
          ? t("dns.toast.nsVerifiedTitle", "Nameservers Verified")
          : t("dns.toast.nsNotVerifiedTitle", "Nameservers not set yet"),
        description: res.ok
          ? res.found.join(", ")
          : t("dns.toast.nsMissing", "Point the domain at: {{servers}}", { servers: res.expected.join(", ") }),
        variant: res.ok ? "default" : "destructive",
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("dns.errors.nsVerifyFailed", "Nameserver verification failed"),
        variant: "destructive",
      });
    } finally {
      setIsVerifyingNs(false);
    }
  };

  return (
    <div className="space-y-6" data-testid="dns-editor-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Globe className="h-6 w-6 text-primary" />
            {t("dns.title", "DNS Zone Editor")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "dns.subtitle",
              "Manage host records, mail routing, verification tokens, and BIND zone exports."
            )}
          </p>
        </div>

        {/* Action Controls */}
        <div className="flex flex-wrap items-center gap-2">
          {/* Zone Selector */}
          <div className="flex items-center gap-1.5">
            <Layers className="h-4 w-4 text-muted-foreground hidden sm:inline" />
            <select
              value={selectedZoneId}
              onChange={(e) => setSelectedZoneId(e.target.value)}
              className="h-9 rounded-md border border-input bg-background px-3 py-1 text-xs font-semibold shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              data-testid="select-active-zone"
            >
              {zones.map((z) => (
                <option key={z.id} value={z.id}>
                  {z.domain_name} ({z.records_count} {t("dns.recordsLabel", "records")})
                </option>
              ))}
            </select>
          </div>

          <Button
            size="sm"
            variant="outline"
            onClick={() => setIsBindOpen(true)}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-bind-export-import"
          >
            <FileText className="h-4 w-4" />
            <span className="hidden sm:inline">{t("dns.actions.bindFile", "BIND Zone")}</span>
          </Button>

          <Button
            size="sm"
            onClick={() => setIsAddOpen(true)}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-add-dns-record"
          >
            <Plus className="h-4 w-4" />
            {t("dns.actions.addRecord", "Add Record")}
          </Button>
        </div>
      </div>

      {/* Nameserver & SOA Info Banner */}
      {activeZone && (
        <Card className="bg-muted/20 border">
          <CardContent className="p-4 flex flex-col md:flex-row md:items-center md:justify-between gap-4">
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Server className="h-4 w-4 text-primary shrink-0" />
                <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                  {t("dns.banner.delegatedNs", "Authoritative Nameservers:")}
                </span>
                <span className="text-xs font-mono font-medium text-foreground">
                  {activeZone.nameservers.join(", ")}
                </span>
              </div>
              <div className="flex items-center gap-4 text-[11px] text-muted-foreground">
                <span>
                  SOA Serial: <strong className="font-mono text-foreground">{activeZone.soa_serial}</strong>
                </span>
                <span className="flex items-center gap-1">
                  DNSSEC:
                  {activeZone.dnssec_enabled ? (
                    <Badge variant="outline" className="text-[10px] h-4 bg-emerald-500/10 text-emerald-600 border-emerald-500/30 gap-1">
                      <ShieldCheck className="h-3 w-3" />
                      {t("dns.banner.signed", "Signed")}
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="text-[10px] h-4 text-muted-foreground">
                      {t("dns.banner.unsigned", "Unsigned")}
                    </Badge>
                  )}
                </span>
              </div>
            </div>

            <Button
              size="sm"
              variant="outline"
              onClick={handleVerifyNs}
              disabled={isVerifyingNs}
              className="text-xs h-8 gap-1.5 shrink-0 self-start md:self-auto"
              data-testid="btn-verify-ns"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isVerifyingNs ? "animate-spin" : ""}`} />
              {t("dns.banner.verifyNsBtn", "Verify NS Resolution")}
            </Button>
          </CardContent>
        </Card>
      )}

      {/* Filter and Search Bar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        {/* Record Type filter pills */}
        <div className="flex items-center gap-1 overflow-x-auto pb-1 sm:pb-0">
          {FILTER_TYPES.map((type) => (
            <Button
              key={type}
              size="sm"
              variant={selectedType === type ? "default" : "outline"}
              className="h-8 text-xs px-2.5 rounded-full"
              onClick={() => setSelectedType(type)}
              data-testid={`btn-filter-type-${type}`}
            >
              {type}
            </Button>
          ))}
        </div>

        {/* Search input */}
        <div className="relative w-full sm:w-64">
          <Search className="h-4 w-4 absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("dns.searchPlaceholder", "Search records...")}
            className="pl-9 h-9 text-xs"
            data-testid="input-search-dns-records"
          />
        </div>
      </div>

      {/* Records Table Card */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-medium flex items-center justify-between">
            <span>
              {t("dns.table.title", "Resource Records")}{" "}
              <span className="text-xs text-muted-foreground font-normal">
                ({filteredRecords.length} {t("dns.table.totalShown", "entries")})
              </span>
            </span>
          </CardTitle>
          <CardDescription>
            {t("dns.table.description", "Click any record to edit directly or add custom routing entries")}
          </CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-1/4">{t("dns.fields.name", "Host / Name")}</TableHead>
                <TableHead className="w-20">{t("dns.fields.type", "Type")}</TableHead>
                <TableHead className="w-1/2">{t("dns.fields.content", "Target / Value")}</TableHead>
                <TableHead className="w-24">{t("dns.fields.ttl", "TTL")}</TableHead>
                <TableHead className="text-right w-20">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={5} className="h-28 text-center text-xs text-muted-foreground">
                    <RefreshCw className="h-5 w-5 animate-spin mx-auto mb-2 text-primary/60" />
                    {t("common.loading", "Loading DNS records...")}
                  </TableCell>
                </TableRow>
              ) : filteredRecords.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="h-28 text-center text-xs text-muted-foreground">
                    {t("dns.table.empty", "No DNS records found for this filter")}
                  </TableCell>
                </TableRow>
              ) : (
                filteredRecords.map((record) => (
                  <InlineRecordRow
                    key={record.id}
                    record={record}
                    domainName={activeZone?.domain_name || ""}
                    isEditing={editingRecordId === record.id}
                    onStartEdit={() => setEditingRecordId(record.id)}
                    onCancelEdit={() => setEditingRecordId(null)}
                    onSaveEdit={(payload) => handleSaveInline(record.id, payload)}
                    onDelete={() => setDeletingRecord(record)}
                  />
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Add Record Dialog */}
      <DnsRecordFormDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        zoneId={selectedZoneId}
        domainName={activeZone?.domain_name || ""}
        onSuccess={(newRec) => {
          setRecords((prev) => [newRec, ...prev]);
          toast({
            title: t("common.success", "Success"),
            description: t("dns.toast.recordAdded", "Record added successfully"),
          });
        }}
      />

      {/* BIND Export/Import Dialog */}
      <BindExportImportDialog
        open={isBindOpen}
        onOpenChange={setIsBindOpen}
        zoneId={selectedZoneId}
        domainName={activeZone?.domain_name || ""}
        records={records}
        nameservers={activeZone?.nameservers ?? []}
        onImportSuccess={() => {
          fetchZoneDetails(selectedZoneId);
        }}
      />

      {/* Delete Record Confirmation Dialog */}
      <Dialog open={!!deletingRecord} onOpenChange={(open) => !open && setDeletingRecord(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="text-destructive flex items-center gap-2">
              {t("dns.deleteDialog.title", "Delete DNS Record")}
            </DialogTitle>
            <DialogDescription className="text-xs pt-1">
              {t(
                "dns.deleteDialog.warning",
                "Are you sure you want to delete this DNS record? This may impact website accessibility or email delivery."
              )}
            </DialogDescription>
          </DialogHeader>

          {deletingRecord && (
            <div className="p-3 rounded-lg border bg-muted/40 font-mono text-xs space-y-1">
              <p>
                <strong>Host:</strong> {deletingRecord.name}
              </p>
              <p>
                <strong>Type:</strong> {deletingRecord.type}
              </p>
              <p className="truncate">
                <strong>Value:</strong> {deletingRecord.content}
              </p>
            </div>
          )}

          <DialogFooter>
            <Button variant="outline" onClick={() => setDeletingRecord(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteRecord}
              data-testid="btn-confirm-delete-record"
            >
              {t("common.delete", "Delete Record")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
