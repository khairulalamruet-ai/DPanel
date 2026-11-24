import React, { useState } from "react";
import {
  TableCell,
  TableRow,
  Badge,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Check, X, Pencil, Trash2 } from "lucide-react";
import type { DnsRecord, DnsRecordType, UpdateDnsRecordPayload } from "./types";

interface InlineRecordRowProps {
  record: DnsRecord;
  domainName: string;
  isEditing: boolean;
  onStartEdit: () => void;
  onCancelEdit: () => void;
  onSaveEdit: (updated: UpdateDnsRecordPayload) => Promise<void>;
  onDelete: () => void;
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

export function InlineRecordRow({
  record,
  domainName,
  isEditing,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onDelete,
}: InlineRecordRowProps) {
  const { t } = useTranslation();

  const [editName, setEditName] = useState(record.name);
  const [editType, setEditType] = useState<DnsRecordType>(record.type);
  const [editContent, setEditContent] = useState(record.content);
  const [editTtl, setEditTtl] = useState(record.ttl);
  const [editPriority, setEditPriority] = useState<number | "">(
    record.priority !== null && record.priority !== undefined ? record.priority : ""
  );
  const [isSaving, setIsSaving] = useState(false);

  const getTypeBadgeClass = (type: DnsRecordType) => {
    switch (type) {
      case "A":
      case "AAAA":
        return "bg-blue-600/90 text-white";
      case "CNAME":
        return "bg-purple-600/90 text-white";
      case "MX":
        return "bg-emerald-600/90 text-white";
      case "TXT":
        return "bg-amber-600/90 text-white";
      case "CAA":
        return "bg-slate-700 text-white";
      case "NS":
        return "bg-cyan-700 text-white";
      default:
        return "bg-secondary text-secondary-foreground";
    }
  };

  const handleSave = async () => {
    try {
      setIsSaving(true);
      await onSaveEdit({
        name: editName.trim(),
        type: editType,
        content: editContent.trim(),
        ttl: Number(editTtl),
        priority: editType === "MX" || editType === "SRV" ? (editPriority !== "" ? Number(editPriority) : null) : null,
      });
    } finally {
      setIsSaving(false);
    }
  };

  if (isEditing) {
    return (
      <TableRow className="bg-primary/5 border-b" data-testid={`inline-edit-row-${record.id}`}>
        {/* Name input */}
        <TableCell className="p-2 align-middle">
          <Input
            value={editName}
            onChange={(e) => setEditName(e.target.value)}
            className="h-8 text-xs font-mono w-full min-w-[80px]"
            data-testid={`input-inline-name-${record.id}`}
          />
        </TableCell>

        {/* Type select */}
        <TableCell className="p-2 align-middle">
          <select
            value={editType}
            onChange={(e) => setEditType(e.target.value as DnsRecordType)}
            className="h-8 rounded-md border border-input bg-background px-2 py-1 text-xs font-semibold shadow-sm"
            data-testid={`select-inline-type-${record.id}`}
          >
            {RECORD_TYPES.map((rt) => (
              <option key={rt} value={rt}>
                {rt}
              </option>
            ))}
          </select>
        </TableCell>

        {/* Content input */}
        <TableCell className="p-2 align-middle">
          <div className="flex items-center gap-1.5">
            {(editType === "MX" || editType === "SRV") && (
              <Input
                type="number"
                placeholder="Prio"
                value={editPriority}
                onChange={(e) => setEditPriority(e.target.value === "" ? "" : Number(e.target.value))}
                className="h-8 text-xs font-mono w-16 shrink-0"
                title="Priority"
                data-testid={`input-inline-priority-${record.id}`}
              />
            )}
            <Input
              value={editContent}
              onChange={(e) => setEditContent(e.target.value)}
              className="h-8 text-xs font-mono w-full min-w-[150px]"
              data-testid={`input-inline-content-${record.id}`}
            />
          </div>
        </TableCell>

        {/* TTL input */}
        <TableCell className="p-2 align-middle">
          <Input
            type="number"
            value={editTtl}
            onChange={(e) => setEditTtl(Number(e.target.value))}
            className="h-8 text-xs font-mono w-20"
            data-testid={`input-inline-ttl-${record.id}`}
          />
        </TableCell>

        {/* Actions Save / Cancel */}
        <TableCell className="p-2 text-right align-middle">
          <div className="flex items-center justify-end gap-1">
            <Button
              size="sm"
              variant="default"
              className="h-7 w-7 p-0 bg-emerald-600 hover:bg-emerald-700 text-white"
              onClick={handleSave}
              disabled={isSaving}
              data-testid={`btn-save-inline-${record.id}`}
              title={t("common.save", "Save")}
            >
              <Check className="h-4 w-4" />
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 w-7 p-0 text-muted-foreground"
              onClick={onCancelEdit}
              disabled={isSaving}
              data-testid={`btn-cancel-inline-${record.id}`}
              title={t("common.cancel", "Cancel")}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </TableCell>
      </TableRow>
    );
  }

  return (
    <TableRow
      className="border-b transition-colors hover:bg-muted/50"
      data-testid={`dns-record-row-${record.id}`}
    >
      {/* Name */}
      <TableCell className="p-3 align-middle font-mono text-xs font-semibold">
        <span className="text-foreground">{record.name}</span>
        {record.name !== "@" && (
          <span className="text-muted-foreground text-[10px] hidden sm:inline">.{domainName}</span>
        )}
      </TableCell>

      {/* Type */}
      <TableCell className="p-3 align-middle">
        <Badge
          className={`text-[10px] font-bold px-2 py-0.5 uppercase tracking-wide ${getTypeBadgeClass(
            record.type
          )}`}
          data-testid={`badge-record-type-${record.id}`}
        >
          {record.type}
        </Badge>
      </TableCell>

      {/* Content / Value */}
      <TableCell className="p-3 align-middle font-mono text-xs max-w-[280px] sm:max-w-[400px]">
        <div className="flex items-center gap-2">
          {record.priority !== null && record.priority !== undefined && (
            <Badge variant="outline" className="text-[10px] h-5 font-mono text-muted-foreground shrink-0">
              {record.priority}
            </Badge>
          )}
          <span className="truncate text-foreground" title={record.content}>
            {record.content}
          </span>
        </div>
      </TableCell>

      {/* TTL */}
      <TableCell className="p-3 align-middle text-xs font-mono text-muted-foreground">
        {record.ttl}s
      </TableCell>

      {/* Actions */}
      <TableCell className="p-3 text-right align-middle">
        <div className="flex items-center justify-end gap-1">
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 p-0 text-muted-foreground hover:text-foreground"
            onClick={onStartEdit}
            data-testid={`btn-edit-record-${record.id}`}
            title={t("common.edit", "Edit Record")}
          >
            <Pencil className="h-3.5 w-3.5" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 p-0 text-destructive/80 hover:text-destructive hover:bg-destructive/10"
            onClick={onDelete}
            data-testid={`btn-delete-record-${record.id}`}
            title={t("common.delete", "Delete Record")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </TableCell>
    </TableRow>
  );
}
