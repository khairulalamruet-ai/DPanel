import { apiFetch } from "@builtpanel/api";
import type { BulkActionPayload, UserItem, UsersListResponse, UserStatus } from "./types";

/** A user as the backend returns it (`UserView`). */
interface UserView {
  id: string;
  username: string;
  email: string;
  locale: string;
  /** `active`, `disabled`, ... */
  account_status: string;
  created_at: string;
  tenant?: {
    id: string;
    username: string;
    node_id: string;
    node_name: string;
    plan_id: string;
    plan_name: string;
    /** `provisioning`, `active`, `suspended`, `deleting`, ... */
    status: string;
    purge_after?: string | null;
  } | null;
}

const statusOf = (u: UserView): UserStatus => {
  const s = u.tenant?.status ?? u.account_status;
  return s === "suspended" || s === "disabled" ? "suspended" : s === "active" ? "active" : "pending";
};

export function toUserItem(u: UserView): UserItem {
  return {
    id: u.id,
    username: u.username,
    email: u.email,
    status: statusOf(u),
    plan_id: u.tenant?.plan_id ?? "",
    plan_name: u.tenant?.plan_name ?? "",
    node_id: u.tenant?.node_id ?? "",
    node_name: u.tenant?.node_name ?? "",
    // Use figures are on the usage page; the list does not carry them.
    disk_used_bytes: 0,
    disk_limit_bytes: -1,
    bandwidth_used_bytes: 0,
    bandwidth_limit_bytes: -1,
    domains_count: 0,
    created_at: u.created_at,
  };
}

export interface UserFilters {
  page: number;
  perPage: number;
  search?: string;
  status?: string;
  planId?: string;
  nodeId?: string;
}

export async function listUsers(f: UserFilters): Promise<UsersListResponse> {
  const q = new URLSearchParams({ limit: String(f.perPage), offset: String((f.page - 1) * f.perPage) });
  if (f.search) q.set("q", f.search);
  // The panel's "active/suspended" is the hosting account's status.
  if (f.status && f.status !== "all") q.set("status", f.status === "pending" ? "provisioning" : f.status);
  if (f.planId && f.planId !== "all") q.set("plan_id", f.planId);
  if (f.nodeId && f.nodeId !== "all") q.set("node_id", f.nodeId);
  const [res, nodes] = await Promise.all([
    apiFetch<{ items: UserView[]; total: number }>(`/api/v1/admin/users?${q}`),
    // The status of each user's node, to warn about the ones that are down.
    apiFetch<{ items: { id: string; status: string }[] }>("/api/v1/admin/nodes").catch(() => ({ items: [] })),
  ]);
  const nodeStatus = new Map(nodes.items.map((n) => [n.id, n.status]));
  return {
    items: res.items.map((u) => {
      const item = toUserItem(u);
      const st = nodeStatus.get(item.node_id);
      if (st === "online" || st === "offline" || st === "draining" || st === "revoked") {
        item.node_status = st;
        item.is_node_offline = st === "offline";
      }
      return item;
    }),
    total: res.total,
    page: f.page,
    per_page: f.perPage,
    total_pages: Math.max(1, Math.ceil(res.total / f.perPage)),
  };
}

const enc = encodeURIComponent;

/** One call per user (the backend has no bulk call); a user that fails does not stop the others. */
export async function bulkAction(p: BulkActionPayload): Promise<{ done: number; failed: { id: string; message: string }[] }> {
  const failed: { id: string; message: string }[] = [];
  let done = 0;
  for (const id of p.user_ids) {
    try {
      if (p.action === "suspend") await apiFetch(`/api/v1/admin/users/${enc(id)}/suspend`, { method: "POST" });
      else if (p.action === "unsuspend") await apiFetch(`/api/v1/admin/users/${enc(id)}/unsuspend`, { method: "POST" });
      else if (p.action === "change_plan") {
        await apiFetch(`/api/v1/admin/users/${enc(id)}/plan`, { method: "POST", body: JSON.stringify({ plan_id: p.plan_id }) });
      } else await apiFetch(`/api/v1/admin/users/${enc(id)}`, { method: "DELETE" });
      done++;
    } catch (err: any) {
      failed.push({ id, message: err?.message ?? "failed" });
    }
  }
  if (done === 0 && failed.length > 0) throw new Error(failed[0].message);
  return { done, failed };
}
