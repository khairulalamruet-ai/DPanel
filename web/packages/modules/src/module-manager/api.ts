import { apiFetch } from "@builtpanel/api";
import type { ModuleItem } from "./types";

/** A module as the backend returns it (`ModuleView`). */
interface ModuleView {
  name: string;
  version: string;
  title: Record<string, string>;
  category: string;
  core: boolean;
  /** Switched on by an administrator. */
  enabled: boolean;
  /** Its routes are being served right now. */
  active: boolean;
  /** Part of the running build. */
  present: boolean;
  depends_on: string[];
  dependents: string[];
}

export function toModule(m: ModuleView): ModuleItem {
  const title = m.title ?? {};
  return {
    name: m.name,
    title: { en: title.en ?? m.name, bn: title.bn ?? title.en ?? m.name },
    description: { en: "", bn: "" },
    version: m.version,
    category: (["core", "web", "db", "files", "mail", "security", "system", "apps"].includes(m.category)
      ? m.category
      : "system") as ModuleItem["category"],
    core: m.core,
    enabled: m.enabled,
    depends_on: m.depends_on ?? [],
    status: !m.present ? "error" : m.active ? "active" : "disabled",
  };
}

export async function listModules(): Promise<ModuleItem[]> {
  const res = await apiFetch<{ items: ModuleView[] }>("/api/v1/admin/modules");
  return res.items.map(toModule);
}

/** Switches a module on or off; with `cascade` what depends on it (or what it needs) changes with it. */
export async function setModuleEnabled(name: string, enabled: boolean, cascade: boolean): Promise<{ changed: string[] }> {
  return apiFetch<{ changed: string[]; active: string[] }>(`/api/v1/admin/modules/${encodeURIComponent(name)}`, {
    method: "PUT",
    body: JSON.stringify({ enabled, cascade }),
  });
}
