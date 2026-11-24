import React, { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Pencil, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { type Cell, type ColumnInfo, type Filter, type FilterOp, type Grid, studio } from "./api";
import { StudioGrid } from "./StudioGrid";
import { RowDialog } from "./RowDialog";

const PAGE = 50;

const OPS: { op: FilterOp; label: string }[] = [
  { op: "eq", label: "=" },
  { op: "ne", label: "≠" },
  { op: "lt", label: "<" },
  { op: "le", label: "≤" },
  { op: "gt", label: ">" },
  { op: "ge", label: "≥" },
  { op: "like", label: "LIKE" },
  { op: "is_null", label: "IS NULL" },
  { op: "not_null", label: "IS NOT NULL" },
];

interface Props {
  databaseId: string;
  table: string;
  /** A view cannot be changed. */
  editable: boolean;
  columns: ColumnInfo[];
}

/** A table's rows: sort, filter, page, add, change and delete. */
export function TableDataView({ databaseId, table, editable, columns }: Props) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [grid, setGrid] = useState<Grid | null>(null);
  const [total, setTotal] = useState(0);
  const [offset, setOffset] = useState(0);
  const [orderBy, setOrderBy] = useState<string | undefined>();
  const [descending, setDescending] = useState(false);
  const [filters, setFilters] = useState<Filter[]>([]);
  const [draft, setDraft] = useState<Filter>({ column: columns[0]?.name ?? "", op: "eq", value: "" });
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Cell[] | null>(null);
  const [deleting, setDeleting] = useState<Cell[] | null>(null);

  const primary = useMemo(() => columns.filter((c) => c.primary_key), [columns]);
  const canChange = editable && primary.length > 0;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await studio(
        databaseId,
        { type: "rows", table, offset, limit: PAGE, order_by: orderBy, descending, filters },
        "rows",
      );
      setGrid(r.grid);
      setTotal(r.total);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [databaseId, table, offset, orderBy, descending, filters]);

  useEffect(() => {
    void load();
  }, [load]);

  const keyOf = (row: Cell[]) =>
    primary.map((c) => ({ column: c.name, value: row[grid!.columns.indexOf(c.name)] ?? "" }));

  const sort = (column: string) => {
    setOffset(0);
    if (orderBy === column) setDescending((d) => !d);
    else {
      setOrderBy(column);
      setDescending(false);
    }
  };

  const addFilter = () => {
    const needsValue = draft.op !== "is_null" && draft.op !== "not_null";
    setFilters((f) => [...f, { column: draft.column, op: draft.op, value: needsValue ? (draft.value ?? "") : undefined }]);
    setDraft((d) => ({ ...d, value: "" }));
    setOffset(0);
  };

  const remove = async () => {
    if (!deleting) return;
    try {
      await studio(databaseId, { type: "delete_row", table, key: keyOf(deleting) }, "changed");
      toast({ title: t("studio.rowDeleted", "Row deleted"), variant: "default" });
      setDeleting(null);
      await load();
    } catch (e) {
      setDeleting(null);
      toast({ title: t("common.error", "Error"), description: (e as Error).message, variant: "destructive" });
    }
  };

  const last = Math.min(offset + PAGE, total);

  return (
    <div className="space-y-3" data-testid="table-data">
      <div className="flex flex-wrap items-end gap-2">
        <select
          aria-label={t("studio.filterColumn", "Filter column")}
          className="h-9 rounded-md border border-input bg-transparent px-2 text-sm"
          value={draft.column}
          onChange={(e) => setDraft({ ...draft, column: e.target.value })}
          data-testid="filter-column"
        >
          {columns.map((c) => (
            <option key={c.name} value={c.name}>
              {c.name}
            </option>
          ))}
        </select>
        <select
          aria-label={t("studio.filterOp", "Filter operator")}
          className="h-9 rounded-md border border-input bg-transparent px-2 text-sm"
          value={draft.op}
          onChange={(e) => setDraft({ ...draft, op: e.target.value as FilterOp })}
          data-testid="filter-op"
        >
          {OPS.map((o) => (
            <option key={o.op} value={o.op}>
              {o.label}
            </option>
          ))}
        </select>
        {draft.op !== "is_null" && draft.op !== "not_null" && (
          <div className="w-48">
            <Input
              aria-label={t("studio.filterValue", "Filter value")}
              value={draft.value ?? ""}
              onChange={(e) => setDraft({ ...draft, value: e.target.value })}
              onKeyDown={(e) => e.key === "Enter" && addFilter()}
              placeholder={t("studio.value", "value")}
              data-testid="filter-value"
            />
          </div>
        )}
        <Button variant="outline" size="sm" onClick={addFilter} disabled={!draft.column} data-testid="filter-add">
          {t("studio.addFilter", "Add filter")}
        </Button>
        <div className="ml-auto flex gap-2">
          <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading} title={t("studio.refresh", "Refresh")}>
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </Button>
          {canChange && (
            <Button size="sm" onClick={() => setAdding(true)} data-testid="row-add">
              <Plus className="mr-1 h-4 w-4" />
              {t("studio.addRow", "Add row")}
            </Button>
          )}
        </div>
      </div>

      {filters.length > 0 && (
        <div className="flex flex-wrap gap-1.5" data-testid="filter-chips">
          {filters.map((f, i) => (
            <span key={i} className="inline-flex items-center gap-1 rounded-full bg-secondary px-2.5 py-0.5 font-mono text-xs">
              {f.column} {OPS.find((o) => o.op === f.op)?.label} {f.value ?? ""}
              <button
                type="button"
                aria-label={t("studio.removeFilter", "Remove filter")}
                onClick={() => {
                  setFilters((all) => all.filter((_, j) => j !== i));
                  setOffset(0);
                }}
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {error && (
        <p role="alert" className="rounded-md bg-destructive/10 p-3 text-sm text-destructive" data-testid="data-error">
          {error}
        </p>
      )}

      {grid && (
        <StudioGrid
          grid={grid}
          sortColumn={orderBy}
          descending={descending}
          onSort={sort}
          actions={
            canChange
              ? (row, index) => (
                  <div className="flex justify-end gap-1">
                    <Button variant="ghost" size="sm" className="h-7 w-7 p-0" title={t("studio.editRow", "Edit row")} onClick={() => setEditing(row)} data-testid={`row-edit-${index}`}>
                      <Pencil className="h-3.5 w-3.5" />
                    </Button>
                    <Button variant="ghost" size="sm" className="h-7 w-7 p-0 text-destructive" title={t("common.delete", "Delete")} onClick={() => setDeleting(row)} data-testid={`row-delete-${index}`}>
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                )
              : undefined
          }
        />
      )}
      {editable && primary.length === 0 && columns.length > 0 && (
        <p className="text-xs text-muted-foreground" data-testid="no-primary-key">
          {t("studio.noPrimaryKey", "This table has no primary key, so single rows cannot be changed here; use the SQL tab.")}
        </p>
      )}

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span data-testid="page-range">
          {total === 0 ? "0" : `${offset + 1}–${last}`} / {total}
        </span>
        <div className="flex gap-1">
          <Button variant="outline" size="sm" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))} data-testid="page-prev">
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <Button variant="outline" size="sm" disabled={offset + PAGE >= total} onClick={() => setOffset(offset + PAGE)} data-testid="page-next">
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <RowDialog
        open={adding}
        onOpenChange={setAdding}
        table={table}
        columns={columns}
        onSave={async (values) => {
          await studio(databaseId, { type: "insert_row", table, values }, "changed");
          toast({ title: t("studio.rowAdded", "Row added"), variant: "default" });
          await load();
        }}
      />
      <RowDialog
        open={editing !== null}
        onOpenChange={(o) => !o && setEditing(null)}
        table={table}
        columns={columns}
        row={editing ?? undefined}
        onSave={async (values) => {
          if (values.length === 0) return;
          await studio(databaseId, { type: "update_row", table, key: keyOf(editing!), values }, "changed");
          toast({ title: t("studio.rowSaved", "Row saved"), variant: "default" });
          await load();
        }}
      />
      <Dialog open={deleting !== null} onOpenChange={(o) => !o && setDeleting(null)}>
        <DialogContent onClose={() => setDeleting(null)}>
          <DialogHeader>
            <DialogTitle>{t("studio.deleteRow", "Delete this row?")}</DialogTitle>
            <DialogDescription>{t("studio.deleteRowHelp", "The row is removed from the table for good.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleting(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button variant="destructive" onClick={() => void remove()} data-testid="row-delete-confirm">
              {t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
