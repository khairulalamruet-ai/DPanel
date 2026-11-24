import { apiFetch } from "@builtpanel/api";
import type {
  AppCategory,
  CatalogApp,
  InstallAppPayload,
  InstalledApp,
  InstalledAppStatus,
  WordPressDetails,
  WordPressScan,
} from "./types";

const A = "/api/v1/client/apps";
const enc = encodeURIComponent;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface CatalogEntry {
  slug: string;
  name: string;
  category: string;
  description: string;
  version: string;
  needs_database: boolean;
  needs_admin: boolean;
}

/** An installed application as the backend returns it (`AppView`). */
interface AppView {
  id: string;
  /** The catalog slug (`wordpress`). */
  kind: string;
  name: string;
  path: string;
  fqdn?: string | null;
  domain_id?: string | null;
  database_id?: string | null;
  version?: string | null;
  /** `installing`, `active`, `failed` or `removing`. */
  status: string;
  error?: string | null;
  /** Unix seconds. */
  created_at: number;
}

const categoryOf = (c: string): AppCategory => (c === "e-commerce" || c === "framework" || c === "static" ? c : "cms");

export function toCatalogApp(e: CatalogEntry): CatalogApp {
  return {
    id: e.slug,
    name: e.name,
    category: categoryOf(e.category),
    version: e.version,
    description: e.description,
    icon: e.slug,
    requires_db: e.needs_database,
    requires_php: true,
  };
}

const statusOf = (s: string): InstalledAppStatus => (s === "active" ? "running" : s === "failed" ? "error" : "installing");

export function toInstalled(a: AppView): InstalledApp {
  const base = a.fqdn ? `https://${a.fqdn}${a.path === "/" ? "" : a.path}` : "";
  return {
    id: a.id,
    catalog_app_id: a.kind,
    name: a.name,
    domain: a.fqdn ?? "",
    path: a.path,
    url: base,
    admin_url: a.kind === "wordpress" && base ? `${base}/wp-admin` : undefined,
    version: a.version ?? "",
    latest_version: a.version ?? "",
    has_update: false,
    status: statusOf(a.status),
    installed_at: new Date(a.created_at * 1000).toISOString(),
    auto_update: false,
  };
}

export async function listCatalog(): Promise<CatalogApp[]> {
  return (await apiFetch<{ items: CatalogEntry[] }>(`${A}/catalog`)).items.map(toCatalogApp);
}

export async function listInstalled(): Promise<InstalledApp[]> {
  return (await apiFetch<{ items: AppView[] }>(A)).items.map(toInstalled);
}

/** Stops tracking an application; its files and database stay in the account. */
export async function forgetApp(appId: string): Promise<void> {
  await apiFetch(`${A}/${encodeURIComponent(appId)}`, { method: "DELETE" });
}

/** Starts an install; the app shows as installing until the node is done. */
export async function installApp(p: InstallAppPayload & { domain_id: string }): Promise<InstalledApp> {
  const common = {
    domain_id: p.domain_id,
    path: p.path ? (p.path.startsWith("/") ? p.path : `/${p.path}`) : "/",
    title: p.site_name,
    admin_user: p.admin_username,
    admin_email: p.admin_email,
    admin_password: p.admin_password,
  };
  const app =
    p.catalog_app_id === "wordpress"
      ? await apiFetch<AppView>(`${A}/wordpress`, { method: "POST", body: JSON.stringify(common) })
      : await apiFetch<AppView>(`${A}/install`, { method: "POST", body: JSON.stringify({ slug: p.catalog_app_id, ...common }) });
  return toInstalled(app);
}

// ---- WordPress

interface WpState {
  state?: string;
  error?: string | null;
  output?: string | null;
  overview?: {
    core: string;
    core_update?: string | null;
    maintenance: boolean;
    plugins: { name: string; status: string; version: string; update_available: boolean; update_version?: string | null }[];
    themes: { name: string; status: string; version: string; update_available: boolean; update_version?: string | null }[];
  } | null;
  scan?: WordPressScan | null;
  operation_id?: string;
}

/** Asks again until the node has answered (`202 { operation_id }` first, then the operation's own answer). */
async function settle(appId: string, first: WpState, tries = 90): Promise<WpState> {
  let cur = first;
  for (let i = 0; i < tries; i++) {
    if (cur.operation_id) {
      await sleep(1000);
      cur = await apiFetch<WpState>(`${A}/${enc(appId)}/operations/${enc(cur.operation_id)}`);
      continue;
    }
    if (cur.state === "running" || cur.state === "queued") {
      await sleep(1000);
      cur = await apiFetch<WpState>(`${A}/${enc(appId)}/operations/${enc(cur.operation_id ?? "")}`).catch(() => cur);
      continue;
    }
    return cur;
  }
  throw new Error("The site is taking too long to answer; try again in a moment");
}

function toDetails(appId: string, s: WpState, previous?: WordPressDetails | null): WordPressDetails {
  const o = s.overview;
  return {
    app_id: appId,
    core_version: o?.core ?? previous?.core_version ?? "",
    latest_core_version: o?.core_update ?? o?.core ?? previous?.latest_core_version ?? "",
    maintenance_mode: o?.maintenance ?? previous?.maintenance_mode ?? false,
    xmlrpc_disabled: s.scan?.xmlrpc_disabled ?? previous?.xmlrpc_disabled ?? false,
    debug_mode: s.scan?.debug ?? previous?.debug_mode ?? false,
    plugins:
      o?.plugins.map((p) => ({
        slug: p.name,
        name: p.name,
        version: p.version,
        latest_version: p.update_version ?? p.version,
        has_update: p.update_available,
        active: p.status === "active" || p.status === "must-use",
        author: "",
      })) ?? previous?.plugins ?? [],
    themes: o?.themes.map((t) => ({ slug: t.name, name: t.name, version: t.version, active: t.status === "active" })) ?? previous?.themes ?? [],
    scan: s.scan ?? previous?.scan ?? null,
  };
}

export async function loadWordPress(appId: string): Promise<WordPressDetails> {
  const first = await apiFetch<WpState>(`${A}/${enc(appId)}/wordpress`);
  const done = await settle(appId, first);
  if (done.state === "failed") throw new Error(done.error || "WordPress could not be read");
  return toDetails(appId, done);
}

async function act(appId: string, path: string, body: unknown, previous: WordPressDetails | null): Promise<WordPressDetails> {
  const first = await apiFetch<WpState>(`${A}/${enc(appId)}/wordpress/${path}`, { method: "POST", body: JSON.stringify(body) });
  const done = await settle(appId, first);
  if (done.state === "failed") throw new Error(done.error || done.output || "The action failed");
  // The answer of an action says what it did; the overview is read again for the new state.
  const fresh = await loadWordPress(appId).catch(() => null);
  return fresh ?? toDetails(appId, done, previous);
}

export const setMaintenance = (appId: string, on: boolean, prev: WordPressDetails | null) => act(appId, "maintenance", { on }, prev);
export const updatePlugin = (appId: string, slug: string | null, prev: WordPressDetails | null) =>
  act(appId, "plugins/update", { slug }, prev);
export const updateCore = (appId: string, prev: WordPressDetails | null) => act(appId, "core/update", {}, prev);

/** Checks the core files against WordPress's checksums and reads the safety settings. */
export async function scanWordPress(appId: string, prev: WordPressDetails | null): Promise<WordPressDetails> {
  const first = await apiFetch<WpState>(`${A}/${enc(appId)}/wordpress/scan`);
  const done = await settle(appId, first);
  if (done.state === "failed") throw new Error(done.error || "The scan failed");
  return toDetails(appId, done, prev);
}

/** Puts the safety settings right: debug off, the file editor off, XML-RPC off, permissions. */
export async function hardenWordPress(appId: string, prev: WordPressDetails | null): Promise<WordPressDetails> {
  await act(appId, "harden", { debug_off: true, file_edit_off: true, xmlrpc_off: true, fix_permissions: true }, prev);
  return scanWordPress(appId, prev);
}
