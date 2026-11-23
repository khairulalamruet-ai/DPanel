import React from "react";
import { Key } from "lucide-react";
import { Badge, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import type { ColumnInfo, IndexInfo } from "./api";

interface Props {
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  createSql: string;
}

/** Columns, indexes and the statement that makes the table. */
export function StructureView({ columns, indexes, createSql }: Props) {
  const { t } = useTranslation();
  return (
    <div className="space-y-5" data-testid="table-structure">
      <section className="space-y-2">
        <h3 className="text-sm font-semibold">{t("studio.columns", "Columns")}</h3>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("studio.name", "Name")}</TableHead>
              <TableHead>{t("studio.type", "Type")}</TableHead>
              <TableHead>{t("studio.nullable", "Null")}</TableHead>
              <TableHead>{t("studio.default", "Default")}</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {columns.map((c) => (
              <TableRow key={c.name} data-testid={`column-${c.name}`}>
                <TableCell className="font-mono text-xs font-semibold">
                  {c.primary_key && <Key className="mr-1 inline h-3 w-3 text-amber-500" />}
                  {c.name}
                </TableCell>
                <TableCell className="font-mono text-xs">{c.data_type}</TableCell>
                <TableCell className="text-xs">{c.nullable ? t("common.yes", "Yes") : t("common.no", "No")}</TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">{c.default ?? "—"}</TableCell>
                <TableCell>{c.generated && <Badge variant="secondary">{t("studio.auto", "auto")}</Badge>}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </section>
      <section className="space-y-2">
        <h3 className="text-sm font-semibold">{t("studio.indexes", "Indexes")}</h3>
        {indexes.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("studio.noIndexes", "No indexes.")}</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("studio.name", "Name")}</TableHead>
                <TableHead>{t("studio.columns", "Columns")}</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {indexes.map((i) => (
                <TableRow key={i.name}>
                  <TableCell className="font-mono text-xs font-semibold">{i.name}</TableCell>
                  <TableCell className="font-mono text-xs">{i.columns.join(", ")}</TableCell>
                  <TableCell>{i.unique && <Badge variant="secondary">{t("studio.unique", "unique")}</Badge>}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>
      <section className="space-y-2">
        <h3 className="text-sm font-semibold">SQL</h3>
        <pre className="overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-xs" data-testid="create-sql">
          {createSql}
        </pre>
      </section>
    </div>
  );
}
