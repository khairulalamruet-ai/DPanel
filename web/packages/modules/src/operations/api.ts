import { apiFetch } from "@builtpanel/api";
import type { OperationItem, OperationStatus } from "./types";

/** An operation as the backend returns it (`OperationView`). */
export interface OperationView {
  id: string;
  idempotency_key: string;
  /** What it does (`ping`, `report_capabilities`, ...). */
  kind: string;
  node_id: string;
  tenant_id?: string | null;
  status: string;
  attempts: number;
  payload: Record<string, unknown>;
  output?: Record<string, unknown> | null;
  error?: { code: string; message: string; retryable?: boolean } | null;
  created_at: string;
  finished_at?: string | null;
  dispatched_at?: string | null;
}

export function toOperation(o: OperationView, nodeNames?: Map<string, string>): OperationItem {
  return {
    id: o.id,
    idempotency_key: o.idempotency_key,
    op_type: o.kind,
    node_id: o.node_id,
    node_name: nodeNames?.get(o.node_id),
    tenant_id: o.tenant_id ?? null,
    status: o.status as OperationStatus,
    retries_count: Math.max(0, o.attempts - 1),
    payload: o.payload ?? {},
    output: o.output ?? null,
    error: o.error ? { code: o.error.code, message: o.error.message } : null,
    created_at: o.created_at,
    updated_at: o.finished_at ?? o.dispatched_at ?? o.created_at,
  };
}

export async function listOperations(filter: { status?: string; kind?: string; nodeId?: string }): Promise<OperationItem[]> {
  const q = new URLSearchParams();
  if (filter.status && filter.status !== "all") q.set("status", filter.status);
  if (filter.kind) q.set("kind", filter.kind);
  if (filter.nodeId) q.set("node_id", filter.nodeId);
  const [ops, nodes] = await Promise.all([
    apiFetch<{ items: OperationView[] }>(`/api/v1/admin/operations${q.toString() ? `?${q}` : ""}`),
    apiFetch<{ items: { id: string; name: string }[] }>("/api/v1/admin/nodes").catch(() => ({ items: [] })),
  ]);
  const names = new Map(nodes.items.map((n) => [n.id, n.name]));
  return ops.items.map((o) => toOperation(o, names));
}

/** Tries a failed operation again; it becomes a new operation (asking twice returns the same one). */
export async function retryOperation(id: string): Promise<OperationItem> {
  return toOperation(await apiFetch<OperationView>(`/api/v1/admin/operations/${encodeURIComponent(id)}/retry`, { method: "POST" }));
}
