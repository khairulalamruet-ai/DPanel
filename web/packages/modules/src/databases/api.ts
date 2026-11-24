import { apiFetch } from "@builtpanel/api";
import type {
  ClientDatabasesResponse,
  DatabaseItem,
  DatabaseUserItem,
} from "./types";

/** `read_only`, `read_write` or `all`; `""` takes the access away. */
export type DbAccess = "" | "read_only" | "read_write" | "all";

interface GrantView {
  user: string;
  user_id: string;
  access: string;
}

interface DatabaseView {
  id: string;
  name: string;
  engine: string;
  status: string;
  size_bytes: number;
  size_checked_at?: string | null;
  users: GrantView[];
}

interface UserGrantView {
  database: string;
  database_id: string;
  access: string;
}

interface DbUserView {
  id: string;
  name: string;
  engine: string;
  status: string;
  databases: UserGrantView[];
  remote_hosts: string[];
}

interface DatabaseList {
  items: DatabaseView[];
  prefix: string;
  limit?: number | null;
}

interface DbUserList {
  items: DbUserView[];
  limit?: number | null;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}

export function toDatabase(d: DatabaseView, prefix: string): DatabaseItem {
  return {
    id: d.id,
    name: d.name,
    raw_name: d.name.startsWith(prefix) ? d.name.slice(prefix.length) : d.name,
    prefix,
    engine: d.engine,
    status: d.status,
    size_bytes: d.size_bytes,
    size_formatted: formatBytes(d.size_bytes),
    assigned_users: d.users.map((u) => u.user),
  };
}

export function toDbUser(u: DbUserView, prefix: string): DatabaseUserItem {
  return {
    id: u.id,
    username: u.name,
    raw_username: u.name.startsWith(prefix) ? u.name.slice(prefix.length) : u.name,
    prefix,
    engine: u.engine,
    status: u.status,
    remote_hosts: u.remote_hosts,
    grants: u.databases.map((g) => ({ db_id: g.database_id, db_name: g.database, access: g.access })),
  };
}

export async function loadDatabases(): Promise<ClientDatabasesResponse> {
  const [dbs, users] = await Promise.all([
    apiFetch<DatabaseList>("/api/v1/client/databases"),
    apiFetch<DbUserList>("/api/v1/client/db-users"),
  ]);
  const databases = dbs.items.map((d) => toDatabase(d, dbs.prefix));
  return {
    databases,
    users: users.items.map((u) => toDbUser(u, dbs.prefix)),
    prefix: dbs.prefix,
    limits: {
      max_databases: dbs.limit ?? -1,
      used_databases: databases.length,
      max_db_users: users.limit ?? -1,
      used_db_users: users.items.length,
      used_db_size_bytes: databases.reduce((sum, d) => sum + d.size_bytes, 0),
    },
  };
}

export async function createDatabase(name: string, prefix: string, engine: "mysql" | "postgres" = "mysql"): Promise<DatabaseItem> {
  const res = await apiFetch<DatabaseView>("/api/v1/client/databases", {
    method: "POST",
    body: JSON.stringify({ name, engine }),
  });
  return toDatabase(res, prefix);
}

export async function deleteDatabase(id: string): Promise<void> {
  await apiFetch(`/api/v1/client/databases/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function createDbUser(input: {
  name: string;
  prefix: string;
  password: string;
  engine?: "mysql" | "postgres";
  grants: { dbId: string; access: DbAccess }[];
}): Promise<DatabaseUserItem> {
  const res = await apiFetch<{ user: DbUserView; password?: string | null }>("/api/v1/client/db-users", {
    method: "POST",
    body: JSON.stringify({ name: input.name, password: input.password, engine: input.engine ?? "mysql" }),
  });
  for (const g of input.grants) {
    if (g.access) await setGrant(g.dbId, res.user.id, g.access);
  }
  return toDbUser(res.user, input.prefix);
}

export async function deleteDbUser(id: string): Promise<void> {
  await apiFetch(`/api/v1/client/db-users/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function changeDbUserPassword(id: string, password: string): Promise<void> {
  await apiFetch(`/api/v1/client/db-users/${encodeURIComponent(id)}/password`, {
    method: "PUT",
    body: JSON.stringify({ password }),
  });
}

/** Gives a user access to a database, or takes it away (`""`). */
export async function setGrant(dbId: string, userId: string, access: DbAccess): Promise<void> {
  const url = `/api/v1/client/databases/${encodeURIComponent(dbId)}/grants/${encodeURIComponent(userId)}`;
  if (access === "") {
    await apiFetch(url, { method: "DELETE" });
  } else {
    await apiFetch(url, { method: "PUT", body: JSON.stringify({ access }) });
  }
}

export async function getRemoteHosts(userId: string): Promise<string[]> {
  const res = await apiFetch<{ cidrs: string[] }>(`/api/v1/client/db-users/${encodeURIComponent(userId)}/remote-hosts`);
  return res.cidrs;
}

export async function setRemoteHosts(userId: string, cidrs: string[]): Promise<string[]> {
  const res = await apiFetch<{ cidrs: string[] }>(`/api/v1/client/db-users/${encodeURIComponent(userId)}/remote-hosts`, {
    method: "PUT",
    body: JSON.stringify({ cidrs }),
  });
  return res.cidrs;
}

/** A short-lived address that opens the database in phpMyAdmin. */
export async function phpMyAdminUrl(dbId: string): Promise<string> {
  const res = await apiFetch<{ url: string; expires_at: string }>(
    `/api/v1/client/databases/${encodeURIComponent(dbId)}/phpmyadmin`,
    { method: "POST" },
  );
  return res.url;
}

interface ExportView {
  id: string;
  status: string;
  error?: string | null;
  size_bytes?: number | null;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Starts a dump, waits until the node has made it and returns a download link (valid five minutes). */
export async function exportDatabase(dbId: string, opts: { intervalMs?: number; tries?: number } = {}): Promise<string> {
  const { intervalMs = 2000, tries = 90 } = opts;
  let exp = await apiFetch<ExportView>(`/api/v1/client/databases/${encodeURIComponent(dbId)}/exports`, { method: "POST" });
  for (let i = 0; i < tries && exp.status !== "ready"; i++) {
    if (exp.status === "failed" || exp.status === "expired") {
      throw new Error(exp.error || "The export failed");
    }
    await sleep(intervalMs);
    exp = await apiFetch<ExportView>(`/api/v1/client/exports/${encodeURIComponent(exp.id)}`);
  }
  if (exp.status !== "ready") throw new Error("The export is taking too long; try again in a moment");
  const link = await apiFetch<{ url: string }>(`/api/v1/client/exports/${encodeURIComponent(exp.id)}/link`, { method: "POST" });
  return link.url;
}

interface ImportView {
  id: string;
  status: string;
  stage?: string | null;
  error?: string | null;
  size_bytes: number;
  received_bytes: number;
  bytes_done: number;
  next_chunk: number;
  max_chunk_bytes: number;
  statements: number;
}

async function sha256Hex(blob: Blob): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Uploads a `.sql` or `.sql.gz` file in chunks, then asks the node to run it and follows it to the end.
 * `onProgress` gets 0..100.
 */
export async function importDatabase(
  dbId: string,
  file: File,
  onProgress: (percent: number) => void,
  opts: { intervalMs?: number } = {},
): Promise<{ statements: number }> {
  const { intervalMs = 2000 } = opts;
  const gzip = /\.gz$/i.test(file.name);
  let imp = await apiFetch<ImportView>(`/api/v1/client/databases/${encodeURIComponent(dbId)}/imports`, {
    method: "POST",
    body: JSON.stringify({ size_bytes: file.size, sha256: await sha256Hex(file), gzip }),
  });
  const chunk = Math.max(1, imp.max_chunk_bytes);
  let n = imp.next_chunk;
  while (n * chunk < file.size) {
    const part = file.slice(n * chunk, Math.min(file.size, (n + 1) * chunk));
    imp = await apiFetch<ImportView>(`/api/v1/client/imports/${encodeURIComponent(imp.id)}/chunks/${n}`, {
      method: "PUT",
      headers: { "Content-Type": "application/octet-stream" },
      body: part,
    });
    n = imp.next_chunk;
    // Uploading is the first half of the bar, the node running the file the second.
    onProgress(Math.min(50, Math.round((imp.received_bytes / file.size) * 50)));
  }
  imp = await apiFetch<ImportView>(`/api/v1/client/imports/${encodeURIComponent(imp.id)}/complete`, { method: "POST" });
  while (imp.status !== "done") {
    if (imp.status === "failed") throw new Error(imp.error || "The import failed");
    onProgress(50 + Math.min(49, Math.round((imp.bytes_done / Math.max(1, file.size)) * 50)));
    await sleep(intervalMs);
    imp = await apiFetch<ImportView>(`/api/v1/client/imports/${encodeURIComponent(imp.id)}`);
  }
  onProgress(100);
  return { statements: imp.statements };
}
