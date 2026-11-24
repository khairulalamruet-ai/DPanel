import React from "react";
import { useTranslation } from "@builtpanel/i18n";
import { Search } from "lucide-react";

interface CommandPaletteTriggerProps {
  onClick: () => void;
}

export function CommandPaletteTrigger({ onClick }: CommandPaletteTriggerProps) {
  const { t } = useTranslation();

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-2 rounded-lg border border-input bg-muted/30 px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors shadow-2xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring max-w-[200px] sm:max-w-[240px] w-full"
      data-testid="command-palette-trigger"
      aria-label={t("search.triggerLabel", "Search or jump to...")}
      title={t("search.triggerTitle", "Global search & command palette (Ctrl+K)")}
    >
      <Search className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate flex-1 text-left">
        {t("search.triggerPlaceholder", "Search...")}
      </span>
      <kbd className="hidden sm:inline-flex items-center gap-0.5 rounded border border-border bg-background px-1 py-0.5 font-mono text-[10px] text-muted-foreground shadow-2xs">
        <span className="text-[11px]">⌘</span>K
      </kbd>
    </button>
  );
}
