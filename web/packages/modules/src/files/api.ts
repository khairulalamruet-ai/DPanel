import { apiFetch } from "@builtpanel/api";
import type { DiskUsageResponse, FileItem, FileListResponse, ReadFileResponse, TrashListResponse } from "./types";

/** An entry as the backend returns it (`EntryView`). */
interface EntryView {
  name: string;
  /** `file`, `dir`, `link` or `other`. */
  kind: string;
  size: number;
  /** The permission bits as the number they make (`0o644` is 420). */
  mode: number;
  modified_at: string;
}

interface Transfer {
  id: string;
  status: string;
  error?: string | null;
  next_chunk: number;
  max_chunk_bytes: number;
  received_bytes: number;
}

const F = "/api/v1/client/files";
const enc = encodeURIComponent;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The panel shows `/public_html/index.php`; the backend takes `public_html/index.php` (`""`: the top). */
export const toApiPath = (path: string): string => path.replace(/^\/+/, "").replace(/\/+$/, "");
const join = (dir: string, name: string): string => `${dir.replace(/\/+$/, "")}/${name}`;

export function formatSize(bytes: number): string {
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

export const modeText = (mode: number): string => `0${(mode & 0o777).toString(8).padStart(3, "0")}`;

export function toFileItem(dir: string, e: EntryView): FileItem {
  const path = join(dir || "/", e.name);
  return {
    id: path,
    name: e.name,
    path,
    type: e.kind === "dir" ? "directory" : e.kind === "link" ? "symlink" : "file",
    size_bytes: e.size,
    size_formatted: formatSize(e.size),
    extension: e.kind === "file" && e.name.includes(".") ? e.name.split(".").pop() : undefined,
    permissions: modeText(e.mode),
    // The backend does not say who owns a file: everything is the account's own.
    owner: "—",
    group: "—",
    updated_at: e.modified_at,
  };
}

/** A folder, every page of it. */
export async function listFolder(path: string): Promise<FileListResponse> {
  const dir = `/${toApiPath(path)}`.replace(/\/$/, "") || "/";
  const items: EntryView[] = [];
  let total = 0;
  do {
    const page = await apiFetch<{ items: EntryView[]; total: number }>(
      `${F}?path=${enc(toApiPath(path))}&offset=${items.length}&limit=1000`,
    );
    items.push(...page.items);
    total = page.total;
    if (page.items.length === 0) break;
  } while (items.length < total);
  const list = items.map((e) => toFileItem(dir, e));
  const parent = dir === "/" ? null : dir.slice(0, dir.lastIndexOf("/")) || "/";
  const bytes = list.reduce((sum, f) => sum + (f.type === "file" ? f.size_bytes : 0), 0);
  return {
    current_path: dir,
    parent_path: parent,
    items: list,
    total_files: list.filter((f) => f.type !== "directory").length,
    total_dirs: list.filter((f) => f.type === "directory").length,
    total_size_bytes: bytes,
    total_size_formatted: formatSize(bytes),
  };
}

/** A request that was answered `202` finishes later: its operation says when. */
async function waitOperation(id: string, tries = 120): Promise<void> {
  for (let i = 0; i < tries; i++) {
    const res = await apiFetch<{ count?: number; operation_id?: string }>(`${F}/operations/${enc(id)}`);
    if (res.count !== undefined) return;
    await sleep(1000);
  }
  throw new Error("The operation is taking too long; look at the folder again in a moment");
}

async function run(path: string, body: unknown, method = "POST"): Promise<void> {
  const res = await apiFetch<{ count?: number; operation_id?: string }>(`${F}${path}`, {
    method,
    body: JSON.stringify(body),
  });
  if (res.operation_id && res.count === undefined) await waitOperation(res.operation_id);
}

export async function readFile(path: string): Promise<ReadFileResponse & { mtime: number; binary: boolean }> {
  const res = await apiFetch<{ content?: string | null; binary: boolean; mtime: number; size: number }>(
    `${F}/content?path=${enc(toApiPath(path))}`,
  );
  if (res.binary) throw new Error("This is not a text file");
  return {
    path,
    content: res.content ?? "",
    encoding: "utf-8",
    size_bytes: res.size,
    mime_type: "text/plain",
    mtime: res.mtime,
    binary: res.binary,
  };
}

/** Saves a text file whole; when `expectedMtime` is given, a change made meanwhile is refused (409). */
export async function writeFile(path: string, content: string, expectedMtime?: number): Promise<void> {
  await run(
    "/content",
    { path: toApiPath(path), content, ...(expectedMtime ? { expected_mtime: expectedMtime } : {}) },
    "PUT",
  );
}

export async function createItem(parent: string, name: string, type: "file" | "directory"): Promise<void> {
  const path = toApiPath(join(parent || "/", name));
  if (type === "directory") await run("/folders", { path });
  else await run("/content", { path, content: "", create_only: true }, "PUT");
}

export async function renameItem(path: string, newName: string): Promise<void> {
  const from = toApiPath(path);
  const slash = from.lastIndexOf("/");
  const to = slash === -1 ? newName : `${from.slice(0, slash)}/${newName}`;
  await run("/move", { from, to });
}

/** The file goes to the trash. */
export async function deleteItem(path: string): Promise<void> {
  await apiFetch(`${F}?path=${enc(toApiPath(path))}`, { method: "DELETE" });
}

export async function chmod(path: string, octal: string, recursive = false): Promise<void> {
  await run("/permissions", { path: toApiPath(path), mode: parseInt(octal, 8), recursive });
}

export async function archive(paths: string[], destFolder: string, name: string, format: "zip" | "tar.gz"): Promise<void> {
  const file = /\.(zip|tar\.gz|tgz)$/i.test(name) ? name : `${name}.${format}`;
  await run("/archive", { paths: paths.map(toApiPath), dest: toApiPath(join(destFolder || "/", file)), format });
}

export async function extract(archivePath: string, dest: string): Promise<void> {
  await run("/extract", { path: toApiPath(archivePath), dest: toApiPath(dest) });
}

export async function listTrash(): Promise<TrashListResponse> {
  const res = await apiFetch<{ items: { id: string; path: string; kind: string; size: number; deleted_at: string }[] }>(`${F}/trash`);
  const items: FileItem[] = res.items.map((t) => {
    const name = t.path.split("/").filter(Boolean).pop() ?? t.path;
    return {
      id: t.id,
      name,
      path: `/${t.path.replace(/^\/+/, "")}`,
      type: t.kind === "dir" ? "directory" : t.kind === "link" ? "symlink" : "file",
      size_bytes: t.size,
      size_formatted: formatSize(t.size),
      permissions: "—",
      owner: "—",
      group: "—",
      updated_at: t.deleted_at,
      in_trash: true,
      original_path: `/${t.path.replace(/^\/+/, "")}`,
    };
  });
  return { items, total_items: items.length };
}

export async function restoreFromTrash(id: string): Promise<void> {
  await apiFetch(`${F}/trash/${enc(id)}/restore`, { method: "POST" });
}

export async function emptyTrash(): Promise<void> {
  await apiFetch(`${F}/trash`, { method: "DELETE" });
}

async function sha256Hex(blob: Blob): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Uploads a file in chunks into a folder and waits until the node has taken it. */
export async function uploadFile(folder: string, file: File, opts: { overwrite?: boolean; intervalMs?: number } = {}): Promise<void> {
  const { overwrite = false, intervalMs = 1000 } = opts;
  let t = await apiFetch<Transfer>(`${F}/uploads`, {
    method: "POST",
    body: JSON.stringify({
      path: toApiPath(join(folder || "/", file.name)),
      size_bytes: file.size,
      sha256: await sha256Hex(file),
      overwrite,
    }),
  });
  const chunk = Math.max(1, t.max_chunk_bytes);
  let n = t.next_chunk;
  while (n * chunk < file.size) {
    t = await apiFetch<Transfer>(`${F}/uploads/${enc(t.id)}/chunks/${n}`, {
      method: "PUT",
      headers: { "Content-Type": "application/octet-stream" },
      body: file.slice(n * chunk, Math.min(file.size, (n + 1) * chunk)),
    });
    n = t.next_chunk;
  }
  t = await apiFetch<Transfer>(`${F}/uploads/${enc(t.id)}/complete`, { method: "POST" });
  while (t.status !== "done") {
    if (t.status === "failed" || t.status === "cancelled" || t.status === "expired") {
      throw new Error(t.error || `The upload ${t.status}`);
    }
    await sleep(intervalMs);
    t = await apiFetch<Transfer>(`${F}/transfers/${enc(t.id)}`);
  }
}

/** Asks the node for the file and returns a link that works for five minutes. */
export async function downloadLink(path: string, intervalMs = 1000, tries = 120): Promise<string> {
  let t = await apiFetch<Transfer>(`${F}/downloads`, { method: "POST", body: JSON.stringify({ path: toApiPath(path) }) });
  for (let i = 0; i < tries && t.status !== "ready"; i++) {
    if (t.status === "failed" || t.status === "cancelled" || t.status === "expired") {
      throw new Error(t.error || `The download ${t.status}`);
    }
    await sleep(intervalMs);
    t = await apiFetch<Transfer>(`${F}/transfers/${enc(t.id)}`);
  }
  if (t.status !== "ready") throw new Error("The file is taking too long to get ready");
  return (await apiFetch<{ url: string }>(`${F}/transfers/${enc(t.id)}/link`, { method: "POST" })).url;
}

interface DiskUsageView {
  folders: { path: string; bytes: number; inodes: number }[];
  inodes: number;
  measured_at?: string | null;
  partial: boolean;
  total_bytes: number;
}

/** What the files use; the plan's limits (disk, inodes) are given by whoever knows them. */
export async function diskUsage(limits: { diskBytes?: number | null; inodes?: number | null } = {}): Promise<DiskUsageResponse> {
  const u = await apiFetch<DiskUsageView>(`${F}/usage`);
  const diskLimit = limits.diskBytes ?? 0;
  const inodeLimit = limits.inodes ?? 0;
  return {
    disk_used_bytes: u.total_bytes,
    disk_used_formatted: formatSize(u.total_bytes),
    disk_limit_bytes: diskLimit,
    disk_limit_formatted: diskLimit ? formatSize(diskLimit) : "∞",
    disk_usage_pct: diskLimit ? Math.round((u.total_bytes / diskLimit) * 100) : 0,
    inodes_used: u.inodes,
    inodes_limit: inodeLimit,
    inodes_usage_pct: inodeLimit ? Math.round((u.inodes / inodeLimit) * 100) : 0,
    last_scanned_at: u.measured_at ?? "",
    partial: u.partial,
    top_directories: u.folders.map((f) => ({
      path: `/${f.path.replace(/^\/+/, "")}`,
      name: f.path.split("/").filter(Boolean).pop() ?? f.path,
      size_bytes: f.bytes,
      size_formatted: formatSize(f.bytes),
      percentage_of_total: u.total_bytes ? Math.round((f.bytes / u.total_bytes) * 100) : 0,
      inodes_count: f.inodes,
      files_count: f.inodes,
      last_calculated_at: u.measured_at ?? "",
    })),
  };
}

/** Measures now (at most once in five minutes); the figures arrive with the next reading. */
export async function remeasure(): Promise<void> {
  await apiFetch(`${F}/usage`, { method: "POST" });
}
