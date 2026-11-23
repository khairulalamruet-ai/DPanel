import { apiFetch } from "@builtpanel/api";

/** A text cell; `null` is SQL NULL. */
export type Cell = string | null;

export interface Grid {
  columns: string[];
  rows: Cell[][];
  truncated: boolean;
}

export interface TableInfo {
  name: string;
  kind: "table" | "view";
  rows: number;
  size_bytes: number;
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  primary_key: boolean;
  default?: string | null;
  generated: boolean;
}

export interface IndexInfo {
  name: string;
  columns: string[];
  unique: boolean;
}

export type FilterOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "like" | "is_null" | "not_null";

export interface Filter {
  column: string;
  op: FilterOp;
  value?: string | null;
}

export interface RowsQuery {
  table: string;
  offset: number;
  limit: number;
  order_by?: string;
  descending?: boolean;
  filters?: Filter[];
}

export type StudioRequest =
  | { type: "tables" }
  | { type: "describe"; table: string }
  | ({ type: "rows" } & RowsQuery)
  | { type: "query"; sql: string }
  | { type: "insert_row"; table: string; values: { column: string; value: Cell }[] }
  | {
      type: "update_row";
      table: string;
      key: { column: string; value: string }[];
      values: { column: string; value: Cell }[];
    }
  | { type: "delete_row"; table: string; key: { column: string; value: string }[] };

export type StudioResult =
  | { type: "tables"; tables: TableInfo[] }
  | { type: "described"; columns: ColumnInfo[]; indexes: IndexInfo[]; create_sql: string }
  | { type: "rows"; grid: Grid; total: number }
  | { type: "query"; grid?: Grid | null; affected: number; elapsed_ms: number }
  | { type: "changed"; affected: number }
  | { type: "error"; code: "invalid" | "sql" | "not_found" | "too_large"; message: string };

/** One request; an answer of `type: "error"` is thrown as an `Error` with the server's words. */
export async function studio<T extends StudioResult["type"]>(
  databaseId: string,
  request: StudioRequest,
  expect: T,
): Promise<Extract<StudioResult, { type: T }>> {
  const result = await apiFetch<StudioResult>(`/api/v1/client/databases/${databaseId}/studio`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
  });
  if (result.type === "error") throw new Error(result.message);
  if (result.type !== expect) throw new Error("Unexpected answer from the server.");
  return result as Extract<StudioResult, { type: T }>;
}

/** Rows to a CSV text (RFC 4180; NULL is an empty field). */
export function toCsv(grid: Grid): string {
  const field = (c: Cell) => {
    if (c === null) return "";
    return /[",\n\r]/.test(c) ? `"${c.replace(/"/g, '""')}"` : c;
  };
  return [grid.columns.map(field), ...grid.rows.map((r) => r.map(field))]
    .map((r) => r.join(","))
    .join("\r\n");
}
