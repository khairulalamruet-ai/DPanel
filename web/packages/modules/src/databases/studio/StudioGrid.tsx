import React from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow, cn } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import type { Grid } from "./api";

interface Props {
  grid: Grid;
  sortColumn?: string;
  descending?: boolean;
  onSort?: (column: string) => void;
  /** Extra cell at the end of each row (edit / delete). */
  actions?: (row: Grid["rows"][number], index: number) => React.ReactNode;
  testId?: string;
}

/** A result set: sticky headers, NULL shown apart from an empty text, long values cut with a title. */
export function StudioGrid({ grid, sortColumn, descending, onSort, actions, testId }: Props) {
  const { t } = useTranslation();
  return (
    <div data-testid={testId ?? "studio-grid"}>
      <Table>
        <TableHeader>
          <TableRow>
            {grid.columns.map((name) => {
              const sorted = sortColumn === name;
              return (
                <TableHead key={name} className="whitespace-nowrap font-mono text-xs">
                  {onSort ? (
                    <button
                      type="button"
                      className="inline-flex items-center gap-1 hover:text-foreground"
                      onClick={() => onSort(name)}
                      data-testid={`sort-${name}`}
                    >
                      {name}
                      {sorted && (descending ? <ArrowDown className="h-3 w-3" /> : <ArrowUp className="h-3 w-3" />)}
                    </button>
                  ) : (
                    name
                  )}
                </TableHead>
              );
            })}
            {actions && <TableHead className="w-20" />}
          </TableRow>
        </TableHeader>
        <TableBody>
          {grid.rows.length === 0 ? (
            <TableRow>
              <TableCell colSpan={grid.columns.length + (actions ? 1 : 0)} className="py-8 text-center text-muted-foreground">
                {t("studio.noRows", "No rows.")}
              </TableCell>
            </TableRow>
          ) : (
            grid.rows.map((row, r) => (
              <TableRow key={r} data-testid={`studio-row-${r}`}>
                {row.map((cell, c) => (
                  <TableCell
                    key={c}
                    title={cell ?? "NULL"}
                    className={cn(
                      "max-w-[22rem] truncate whitespace-nowrap font-mono text-xs",
                      cell === null && "italic text-muted-foreground",
                    )}
                  >
                    {cell === null ? "NULL" : cell}
                  </TableCell>
                ))}
                {actions && <TableCell className="text-right">{actions(row, r)}</TableCell>}
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
      {grid.truncated && (
        <p className="mt-2 text-xs text-amber-600" data-testid="studio-truncated">
          {t("studio.truncated", "Only part of the result is shown (too many rows or a very long value).")}
        </p>
      )}
    </div>
  );
}
