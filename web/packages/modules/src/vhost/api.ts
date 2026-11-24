import { apiFetch } from "@builtpanel/api";
import type { ServerType, VhostTemplate, VhostTemplateVersion } from "./types";

/** A template with its text (`TemplateView`). */
interface TemplateView {
  id: string;
  name: string;
  /** `nginx` or `caddy`. */
  server: string;
  body: string;
  version: number;
  is_default: boolean;
  updated_at: string;
}

const B = "/api/v1/admin/vhost-templates";
const enc = encodeURIComponent;

/** The `{{name}}` places a template fills in. */
const variablesOf = (body: string): string[] => Array.from(new Set(Array.from(body.matchAll(/\{\{\s*([\w.]+)\s*\}\}/g), (m) => m[1])));

export function toTemplate(t: TemplateView): VhostTemplate {
  return {
    id: t.id,
    name: t.name,
    slug: t.name.toLowerCase().replace(/[^a-z0-9]+/g, "-"),
    server_type: (t.server === "nginx" ? "nginx" : "caddy") as ServerType,
    content: t.body,
    variables: variablesOf(t.body),
    version: t.version,
    is_default: t.is_default,
    domain_count: 0,
    created_at: t.updated_at,
    updated_at: t.updated_at,
  };
}

/** The list has no text, so each template is read for its text. */
export async function listTemplates(): Promise<VhostTemplate[]> {
  const list = await apiFetch<{ items: { id: string }[] }>(B);
  const full = await Promise.all(list.items.map((i) => apiFetch<TemplateView>(`${B}/${enc(i.id)}`)));
  return full.map(toTemplate);
}

export async function saveTemplate(t: VhostTemplate, body: string): Promise<VhostTemplate> {
  return toTemplate(await apiFetch<TemplateView>(`${B}/${enc(t.id)}`, { method: "PUT", body: JSON.stringify({ name: t.name, body }) }));
}

export async function listVersions(id: string): Promise<VhostTemplateVersion[]> {
  const res = await apiFetch<{ items: { version: number; name: string; body: string; changed_by?: string | null; created_at: string }[] }>(
    `${B}/${enc(id)}/versions`,
  );
  return res.items.map((v) => ({
    id: String(v.version),
    template_id: id,
    version: v.version,
    content: v.body,
    changelog: v.name,
    created_by: v.changed_by ?? "",
    created_at: v.created_at,
  }));
}

/** Going back to an earlier text is a new save of that text (the next version). */
export async function revertTo(t: VhostTemplate, v: VhostTemplateVersion): Promise<VhostTemplate> {
  return saveTemplate(t, v.content);
}

/** Sends every account's web server files again, after templates changed. */
export async function applyToAll(): Promise<{ tenants: number }> {
  return apiFetch<{ tenants: number }>(`${B}/apply`, { method: "POST" });
}

export async function previewTemplate(body: string): Promise<string> {
  return (await apiFetch<{ rendered: string }>(`${B}/preview`, { method: "POST", body: JSON.stringify({ body }) })).rendered;
}
