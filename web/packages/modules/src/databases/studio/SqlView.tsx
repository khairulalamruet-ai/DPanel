import React, { useRef, useState } from "react";
import { Download, History, Play } from "lucide-react";
import { Button } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { type Grid, studio, toCsv } from "./api";
import { StudioGrid } from "./StudioGrid";

interface Outcome {
  grid?: Grid | null;
  affected: number;
  elapsed_ms: number;
}

interface Props {
  databaseId: string;
  databaseName: string;
  /** Called after a statement ran, so the table list (a CREATE or a DROP) can be read again. */
  onRan: () => void;
}

/** The SQL editor: run a statement (Ctrl/Cmd + Enter), see the rows, take them as CSV. */
export function SqlView({ databaseId, databaseName, onRan }: Props) {
  const { t } = useTranslation();
  const [sql, setSql] = useState("");
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Kept in this page only, never stored: a statement may hold a password.
  const [history, setHistory] = useState<string[]>([]);
  const area = useRef<HTMLTextAreaElement>(null);

  const run = async () => {
    const text = sql.trim();
    if (!text || running) return;
    setRunning(true);
    setError(null);
    try {
      const r = await studio(databaseId, { type: "query", sql: text }, "query");
      setOutcome({ grid: r.grid, affected: r.affected, elapsed_ms: r.elapsed_ms });
      setHistory((h) => [text, ...h.filter((x) => x !== text)].slice(0, 20));
      onRan();
    } catch (e) {
      setOutcome(null);
      setError((e as Error).message);
    } finally {
      setRunning(false);
    }
  };

  const download = () => {
    if (!outcome?.grid) return;
    const blob = new Blob([toCsv(outcome.grid)], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${databaseName}-result.csv`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  return (
    <div className="space-y-3" data-testid="sql-view">
      <textarea
        ref={area}
        value={sql}
        onChange={(e) => setSql(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void run();
          }
        }}
        spellCheck={false}
        rows={8}
        placeholder="SELECT * FROM …"
        aria-label="SQL"
        className="w-full resize-y rounded-lg border border-input bg-transparent p-3 font-mono text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        data-testid="sql-input"
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={() => void run()} disabled={running || !sql.trim()} data-testid="sql-run">
          <Play className="mr-1 h-4 w-4" />
          {running ? t("studio.running", "Running…") : t("studio.run", "Run")}
        </Button>
        <span className="text-xs text-muted-foreground">{t("studio.runHint", "Ctrl/Cmd + Enter")}</span>
        {outcome?.grid && (
          <Button variant="outline" size="sm" className="ml-auto" onClick={download} data-testid="sql-csv">
            <Download className="mr-1 h-4 w-4" />
            CSV
          </Button>
        )}
      </div>

      {error && (
        <p role="alert" className="rounded-md bg-destructive/10 p-3 font-mono text-xs text-destructive" data-testid="sql-error">
          {error}
        </p>
      )}
      {outcome && (
        <div className="space-y-2" data-testid="sql-outcome">
          <p className="text-xs text-muted-foreground">
            {outcome.grid
              ? t("studio.rowsIn", "{{count}} rows in {{ms}} ms", { count: outcome.grid.rows.length, ms: outcome.elapsed_ms })
              : t("studio.affected", "{{count}} rows affected in {{ms}} ms", { count: outcome.affected, ms: outcome.elapsed_ms })}
          </p>
          {outcome.grid && <StudioGrid grid={outcome.grid} testId="sql-grid" />}
        </div>
      )}

      {history.length > 0 && (
        <details className="rounded-lg border p-3 text-xs">
          <summary className="flex cursor-pointer items-center gap-1 font-medium">
            <History className="h-3.5 w-3.5" />
            {t("studio.history", "This session’s statements")}
          </summary>
          <ul className="mt-2 space-y-1">
            {history.map((h) => (
              <li key={h}>
                <button
                  type="button"
                  className="block w-full truncate rounded px-2 py-1 text-left font-mono hover:bg-muted"
                  onClick={() => {
                    setSql(h);
                    area.current?.focus();
                  }}
                >
                  {h}
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}
