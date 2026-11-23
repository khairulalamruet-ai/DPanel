import React, { useEffect, useState } from "react";
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import type { Cell, ColumnInfo } from "./api";

type Mode = "value" | "null" | "default";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  table: string;
  columns: ColumnInfo[];
  /** The row being changed (same order as `columns`); absent when adding. */
  row?: Cell[];
  onSave: (values: { column: string; value: Cell }[]) => Promise<void>;
}

/** Adds a row or changes one: a field per column, with NULL (and the default, when adding) one click away. */
export function RowDialog({ open, onOpenChange, table, columns, row, onSave }: Props) {
  const { t } = useTranslation();
  const editing = row !== undefined;
  const [values, setValues] = useState<string[]>([]);
  const [modes, setModes] = useState<Mode[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setValues(columns.map((_, i) => (row ? (row[i] ?? "") : "")));
    setModes(
      columns.map((c, i): Mode => {
        if (row) return row[i] === null ? "null" : "value";
        return c.generated || c.default != null || c.nullable ? "default" : "value";
      }),
    );
  }, [open, columns, row]);

  const save = async () => {
    const out: { column: string; value: Cell }[] = [];
    columns.forEach((c, i) => {
      if (modes[i] === "default") return;
      // An unchanged value of an edit is not sent again.
      if (editing && modes[i] === (row![i] === null ? "null" : "value") && (modes[i] === "null" || values[i] === row![i])) return;
      out.push({ column: c.name, value: modes[i] === "null" ? null : values[i] });
    });
    setSaving(true);
    setError(null);
    try {
      await onSave(out);
      onOpenChange(false);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-h-[85vh] max-w-xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {editing ? t("studio.editRow", "Edit row") : t("studio.addRow", "Add row")} — <span className="font-mono">{table}</span>
          </DialogTitle>
          <DialogDescription>
            {editing
              ? t("studio.editRowHelp", "Change the values and save. Only what you changed is sent.")
              : t("studio.addRowHelp", "Columns left on “default” take their default value.")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3 py-2">
          {columns.map((c, i) => (
            <div key={c.name} className="space-y-1">
              <div className="flex items-center justify-between gap-2">
                <label htmlFor={`row-${c.name}`} className="text-xs font-medium">
                  <span className="font-mono">{c.name}</span>{" "}
                  <span className="text-muted-foreground">
                    {c.data_type}
                    {c.primary_key ? " · PK" : ""}
                  </span>
                </label>
                <select
                  aria-label={`${c.name} mode`}
                  className="h-7 rounded-md border border-input bg-transparent px-1 text-xs"
                  value={modes[i] ?? "value"}
                  onChange={(e) => setModes((m) => m.map((x, j) => (j === i ? (e.target.value as Mode) : x)))}
                  data-testid={`mode-${c.name}`}
                >
                  <option value="value">{t("studio.modeValue", "Value")}</option>
                  {c.nullable && <option value="null">NULL</option>}
                  {!editing && <option value="default">{t("studio.modeDefault", "Default")}</option>}
                </select>
              </div>
              <textarea
                id={`row-${c.name}`}
                rows={1}
                disabled={modes[i] !== "value"}
                value={modes[i] === "value" ? (values[i] ?? "") : ""}
                onChange={(e) => setValues((v) => v.map((x, j) => (j === i ? e.target.value : x)))}
                className="min-h-9 w-full resize-y rounded-md border border-input bg-transparent px-3 py-1.5 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
                data-testid={`field-${c.name}`}
              />
            </div>
          ))}
        </div>
        {error && (
          <p role="alert" className="rounded-md bg-destructive/10 p-2 text-xs text-destructive" data-testid="row-error">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel", "Cancel")}
          </Button>
          <Button onClick={() => void save()} disabled={saving} data-testid="row-save">
            {saving ? t("common.loading", "Loading...") : t("common.save", "Save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
