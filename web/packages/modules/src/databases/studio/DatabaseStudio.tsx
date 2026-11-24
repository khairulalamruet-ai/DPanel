import React, { useCallback, useEffect, useMemo, useState } from "react";
import { ArrowLeft, Database, Eye, RefreshCw, Table2 } from "lucide-react";
import { Button, Input, cn } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { formatBytes } from "../api";
import { type ColumnInfo, type IndexInfo, type TableInfo, studio } from "./api";
import { TableDataView } from "./TableDataView";
import { StructureView } from "./StructureView";
import { SqlView } from "./SqlView";

type Tab = "data" | "structure" | "sql";

interface Props {
  database: { id: string; name: string };
  onClose: () => void;
}

interface Described {
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  create_sql: string;
}

/** The database studio: tables on the left; a table's rows and structure, or the SQL editor, on the right. */
export function DatabaseStudio({ database, onClose }: Props) {
  const { t } = useTranslation();
  const [tables, setTables] = useState<TableInfo[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("data");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [described, setDescribed] = useState<Described | null>(null);

  const loadTables = useCallback(async () => {
    try {
      const r = await studio(database.id, { type: "tables" }, "tables");
      setTables(r.tables);
      setError(null);
      setSelected((current) => (current && r.tables.some((x) => x.name === current) ? current : null));
    } catch (e) {
      setError((e as Error).message);
    }
  }, [database.id]);

  useEffect(() => {
    void loadTables();
  }, [loadTables]);

  useEffect(() => {
    setDescribed(null);
    if (!selected) return;
    let live = true;
    studio(database.id, { type: "describe", table: selected }, "described")
      .then((r) => live && setDescribed({ columns: r.columns, indexes: r.indexes, create_sql: r.create_sql }))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [database.id, selected]);

  const shown = useMemo(() => {
    const q = search.trim().toLowerCase();
    return (tables ?? []).filter((x) => !q || x.name.toLowerCase().includes(q));
  }, [tables, search]);
  const current = tables?.find((x) => x.name === selected);

  const tabs: { id: Tab; label: string; needsTable: boolean }[] = [
    { id: "data", label: t("studio.tabData", "Data"), needsTable: true },
    { id: "structure", label: t("studio.tabStructure", "Structure"), needsTable: true },
    { id: "sql", label: "SQL", needsTable: false },
  ];

  return (
    <div className="space-y-4" data-testid="database-studio">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="sm" onClick={onClose} data-testid="studio-back">
          <ArrowLeft className="mr-1 h-4 w-4" />
          {t("studio.back", "Databases")}
        </Button>
        <h1 className="flex items-center gap-2 font-mono text-lg font-semibold" data-testid="studio-title">
          <Database className="h-5 w-5 text-primary" />
          {database.name}
        </h1>
      </div>

      {error && (
        <p role="alert" className="rounded-md bg-destructive/10 p-3 text-sm text-destructive" data-testid="studio-error">
          {error}
        </p>
      )}

      <div className="grid gap-4 md:grid-cols-[16rem_1fr]">
        <aside className="space-y-2 rounded-lg border p-2 md:max-h-[75vh] md:overflow-auto">
          <div className="flex items-center gap-1">
            <Input
              aria-label={t("studio.searchTables", "Search tables")}
              placeholder={t("studio.searchTables", "Search tables")}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              data-testid="table-search"
            />
            <Button variant="ghost" size="sm" className="h-9 w-9 shrink-0 p-0" onClick={() => void loadTables()} title={t("studio.refresh", "Refresh")}>
              <RefreshCw className="h-4 w-4" />
            </Button>
          </div>
          {tables === null ? (
            <p className="p-2 text-xs text-muted-foreground">{t("common.loading", "Loading...")}</p>
          ) : shown.length === 0 ? (
            <p className="p-2 text-xs text-muted-foreground" data-testid="no-tables">
              {tables.length === 0 ? t("studio.noTables", "No tables yet. Make one in the SQL tab.") : t("studio.noMatch", "No match.")}
            </p>
          ) : (
            <ul className="space-y-0.5" data-testid="table-list">
              {shown.map((x) => (
                <li key={x.name}>
                  <button
                    type="button"
                    onClick={() => {
                      setSelected(x.name);
                      if (tab === "sql") setTab("data");
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted",
                      selected === x.name && "bg-muted font-semibold",
                    )}
                    data-testid={`table-${x.name}`}
                  >
                    {x.kind === "view" ? <Eye className="h-3.5 w-3.5 shrink-0 text-muted-foreground" /> : <Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />}
                    <span className="truncate font-mono text-xs">{x.name}</span>
                    <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">{x.kind === "view" ? "view" : `~${x.rows}`}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </aside>

        <main className="min-w-0 space-y-3">
          <div className="flex items-center gap-1 border-b">
            {tabs.map((x) => (
              <button
                key={x.id}
                type="button"
                disabled={x.needsTable && !selected}
                onClick={() => setTab(x.id)}
                className={cn(
                  "-mb-px border-b-2 px-3 py-2 text-sm disabled:opacity-40",
                  tab === x.id ? "border-primary font-semibold" : "border-transparent text-muted-foreground hover:text-foreground",
                )}
                data-testid={`tab-${x.id}`}
              >
                {x.label}
              </button>
            ))}
            {current && tab !== "sql" && (
              <span className="ml-auto pb-2 text-xs text-muted-foreground" data-testid="table-meta">
                <span className="font-mono">{current.name}</span> · {formatBytes(current.size_bytes)}
              </span>
            )}
          </div>

          {tab === "sql" ? (
            <SqlView databaseId={database.id} databaseName={database.name} onRan={() => void loadTables()} />
          ) : !selected || !current ? (
            <p className="py-10 text-center text-sm text-muted-foreground" data-testid="pick-table">
              {t("studio.pickTable", "Pick a table on the left.")}
            </p>
          ) : !described ? (
            <p className="py-10 text-center text-sm text-muted-foreground">{t("common.loading", "Loading...")}</p>
          ) : tab === "data" ? (
            <TableDataView key={selected} databaseId={database.id} table={selected} editable={current.kind === "table"} columns={described.columns} />
          ) : (
            <StructureView columns={described.columns} indexes={described.indexes} createSql={described.create_sql} />
          )}
        </main>
      </div>
    </div>
  );
}
