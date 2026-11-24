import React, { useState, useEffect, useRef, useMemo } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Search,
  Command,
  Globe,
  Users,
  Database,
  Folder,
  Server,
  Lock,
  Archive,
  Clock,
  ShieldAlert,
  Settings,
  ArrowRight,
  CornerDownLeft,
  X,
  FileText,
  Activity,
  Terminal,
} from "lucide-react";
import { SearchResultItem, SearchCategory } from "./types";

interface CommandPaletteProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  onNavigate?: (href: string) => void;
}

export function CommandPalette({
  open: controlledOpen,
  onOpenChange,
  onNavigate,
}: CommandPaletteProps) {
  const { t } = useTranslation();
  const [internalOpen, setInternalOpen] = useState(false);
  const isOpen = controlledOpen !== undefined ? controlledOpen : internalOpen;

  const setOpen = (val: boolean) => {
    if (onOpenChange) onOpenChange(val);
    else setInternalOpen(val);
  };

  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [remoteResults, setRemoteResults] = useState<SearchResultItem[]>([]);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Global Ctrl+K / Cmd+K and Escape shortcut listener
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!isOpen);
      } else if (e.key === "Escape" && isOpen) {
        e.preventDefault();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen]);

  // Focus input when opened
  useEffect(() => {
    if (isOpen) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [isOpen]);

  // Default navigable pages
  const staticPages: SearchResultItem[] = useMemo(
    () => [
      { id: "page-dash", title: t("common.dashboard", "Dashboard"), category: "pages", href: "/" },
      { id: "page-domains", title: t("common.domains", "Domains & Vhosts"), subtitle: "/domains", category: "pages", href: "/domains" },
      { id: "page-dns", title: t("dns.nav", "DNS Zone Editor"), subtitle: "/dns", category: "pages", href: "/dns" },
      { id: "page-ssl", title: t("ssl.nav", "SSL / TLS Certificates"), subtitle: "/ssl", category: "pages", href: "/ssl" },
      { id: "page-users", title: t("common.users", "Users & Tenant Accounts"), subtitle: "/users", category: "pages", href: "/users" },
      { id: "page-databases", title: t("databases.nav", "MySQL Databases & Users"), subtitle: "/databases", category: "pages", href: "/databases" },
      { id: "page-files", title: t("files.nav", "File Manager"), subtitle: "/files", category: "pages", href: "/files" },
      { id: "page-backups", title: t("backups.nav", "Backups & Snapshots"), subtitle: "/backups", category: "pages", href: "/backups" },
      { id: "page-cron", title: t("cron.nav", "Cron Jobs"), subtitle: "/cron", category: "pages", href: "/cron" },
      { id: "page-waf", title: t("waf.nav", "Security & WAF"), subtitle: "/waf", category: "pages", href: "/waf" },
      { id: "page-nodes", title: t("nodes.nav", "Cluster Nodes"), subtitle: "/nodes", category: "pages", href: "/nodes" },
      { id: "page-settings", title: t("common.settings", "System Settings"), subtitle: "/settings", category: "pages", href: "/settings" },
      { id: "page-license", title: t("license.nav", "License Authority"), subtitle: "/admin/license", category: "pages", href: "/admin/license" },
      { id: "page-audit", title: t("audit.nav", "Audit Log Viewer"), subtitle: "/audit", category: "pages", href: "/audit" },
      { id: "page-tools", title: t("serverTools.nav", "Server Diagnostic Tools"), subtitle: "/server-tools", category: "pages", href: "/server-tools" },
    ],
    [t]
  );

  // Search remote domains and users when query has 2+ characters
  useEffect(() => {
    if (!query.trim() || query.length < 2) {
      setRemoteResults([]);
      return;
    }

    let isMounted = true;
    const fetchRemote = async () => {
      setLoading(true);
      try {
        const res = await apiFetch<SearchResultItem[]>(
          `/api/v1/search?q=${encodeURIComponent(query.trim())}`
        );
        if (isMounted) setRemoteResults(res);
      } catch {
        if (isMounted) setRemoteResults([]);
      } finally {
        if (isMounted) setLoading(false);
      }
    };

    const timer = setTimeout(fetchRemote, 150);
    return () => {
      isMounted = false;
      clearTimeout(timer);
    };
  }, [query]);

  // Combined and filtered search items
  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      return staticPages.slice(0, 8);
    }

    const matchedPages = staticPages.filter(
      (p) => p.title.toLowerCase().includes(q) || (p.subtitle && p.subtitle.toLowerCase().includes(q))
    );

    return [...matchedPages, ...remoteResults];
  }, [query, staticPages, remoteResults]);

  // Adjust selectedIndex bounds
  useEffect(() => {
    setSelectedIndex(0);
  }, [filteredItems.length]);

  const handleSelect = (item: SearchResultItem) => {
    setOpen(false);
    if (item.onSelect) {
      item.onSelect();
    } else if (item.href) {
      if (onNavigate) {
        onNavigate(item.href);
      } else {
        window.location.href = item.href;
      }
    }
  };

  const handleInputKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev + 1 < filteredItems.length ? prev + 1 : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev > 0 ? prev - 1 : filteredItems.length - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filteredItems[selectedIndex]) {
        handleSelect(filteredItems[selectedIndex]);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      setOpen(false);
    }
  };

  if (!isOpen) return null;

  const getIcon = (category: SearchCategory) => {
    switch (category) {
      case "domains":
        return <Globe className="h-4 w-4 text-primary shrink-0" />;
      case "users":
        return <Users className="h-4 w-4 text-emerald-500 shrink-0" />;
      case "actions":
        return <Command className="h-4 w-4 text-amber-500 shrink-0" />;
      case "pages":
      default:
        return <FileText className="h-4 w-4 text-muted-foreground shrink-0" />;
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-16 sm:pt-24 p-4"
      data-testid="command-palette-backdrop"
      onClick={() => setOpen(false)}
    >
      {/* Backdrop blur */}
      <div className="fixed inset-0 bg-black/50 backdrop-blur-xs" aria-hidden="true" />

      {/* Modal Dialog */}
      <div
        className="relative z-50 w-full max-w-xl rounded-xl border bg-card text-card-foreground shadow-2xl overflow-hidden flex flex-col animate-in fade-in-0 zoom-in-95 duration-150"
        data-testid="command-palette-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search Header Input */}
        <div className="flex items-center px-4 border-b bg-muted/20">
          <Search className="h-4 w-4 text-muted-foreground mr-3 shrink-0" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder={t("search.inputPlaceholder", "Search users, domains, pages, settings (Ctrl+K)...")}
            className="w-full bg-transparent py-3.5 text-sm outline-none placeholder:text-muted-foreground"
            data-testid="command-palette-input"
          />
          {query && (
            <button
              type="button"
              onClick={() => setQuery("")}
              className="p-1 rounded text-muted-foreground hover:text-foreground text-xs"
              data-testid="btn-clear-search-query"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
          <kbd className="hidden sm:inline-flex items-center gap-0.5 rounded border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground ml-2">
            ESC
          </kbd>
        </div>

        {/* Results List */}
        <div
          ref={listRef}
          className="max-h-80 overflow-y-auto p-2 space-y-1 divide-y divide-border/20"
          data-testid="command-palette-results"
        >
          {filteredItems.length === 0 ? (
            <div
              className="py-10 text-center text-xs text-muted-foreground space-y-1"
              data-testid="command-palette-empty"
            >
              <Search className="h-8 w-8 mx-auto text-muted-foreground/30 stroke-1" />
              <p className="font-semibold text-foreground">
                {t("search.noResultsTitle", "No results found")}
              </p>
              <p className="text-[11px]">
                {t("search.noResultsDesc", "No matching pages, domains, or users found for '{{query}}'.", { query })}
              </p>
            </div>
          ) : (
            filteredItems.map((item, idx) => {
              const isSelected = idx === selectedIndex;
              return (
                <div
                  key={item.id}
                  onClick={() => handleSelect(item)}
                  onMouseEnter={() => setSelectedIndex(idx)}
                  className={`flex items-center justify-between p-2.5 rounded-lg text-xs cursor-pointer transition-colors ${
                    isSelected
                      ? "bg-primary text-primary-foreground font-semibold"
                      : "hover:bg-muted/50 text-foreground"
                  }`}
                  data-testid={`search-result-item-${item.id}`}
                >
                  <div className="flex items-center gap-2.5 truncate">
                    <div className={isSelected ? "text-primary-foreground" : ""}>
                      {getIcon(item.category)}
                    </div>
                    <span className="truncate">{item.title}</span>
                    {item.subtitle && (
                      <span
                        className={`text-[11px] font-mono truncate ${
                          isSelected ? "text-primary-foreground/80" : "text-muted-foreground"
                        }`}
                      >
                        {item.subtitle}
                      </span>
                    )}
                  </div>

                  <div className="flex items-center gap-1.5 shrink-0 ml-2">
                    {item.badge && (
                      <span
                        className={`text-[10px] uppercase font-mono px-1.5 py-0.5 rounded border ${
                          isSelected
                            ? "border-primary-foreground/30 bg-primary-foreground/10 text-primary-foreground"
                            : "border-border bg-muted text-muted-foreground"
                        }`}
                      >
                        {item.badge}
                      </span>
                    )}
                    {isSelected && <CornerDownLeft className="h-3.5 w-3.5 opacity-80" />}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Footer Navigation Hints */}
        <div className="px-4 py-2 border-t bg-muted/15 flex items-center justify-between text-[11px] text-muted-foreground">
          <div className="flex items-center gap-3">
            <span className="flex items-center gap-1">
              <kbd className="rounded border bg-background px-1 text-[10px] font-mono">↑</kbd>
              <kbd className="rounded border bg-background px-1 text-[10px] font-mono">↓</kbd>
              <span>{t("search.navigateHint", "to navigate")}</span>
            </span>
            <span className="flex items-center gap-1">
              <kbd className="rounded border bg-background px-1 text-[10px] font-mono">↵</kbd>
              <span>{t("search.selectHint", "to select")}</span>
            </span>
          </div>

          <span className="flex items-center gap-1">
            <kbd className="rounded border bg-background px-1 text-[10px] font-mono">ESC</kbd>
            <span>{t("search.closeHint", "to close")}</span>
          </span>
        </div>
      </div>
    </div>
  );
}
