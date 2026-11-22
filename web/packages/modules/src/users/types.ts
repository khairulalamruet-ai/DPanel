export type UserStatus = "active" | "suspended" | "pending";

export interface UserItem {
  id: string;
  username: string;
  email: string;
  status: UserStatus;
  plan_id: string;
  plan_name: string;
  node_id: string;
  node_name: string;
  node_status?: "online" | "offline" | "draining" | "revoked";
  is_node_offline?: boolean;
  primary_domain?: string;
  disk_used_bytes: number;
  disk_limit_bytes: number; // -1 for unlimited
  bandwidth_used_bytes: number;
  bandwidth_limit_bytes: number; // -1 for unlimited
  domains_count: number;
  created_at: string;
  suspended_at?: string | null;
  suspended_reason?: string | null;
  /** Whole percent of the plan limit in use; null where there is no limit or no reading. */
  usage?: {
    memory_percent: number | null;
    cpu_percent: number | null;
    disk_percent: number | null;
    inodes_percent: number | null;
  };
}

export interface UsersListResponse {
  items: UserItem[];
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
}

export interface UsersFilterParams {
  page?: number;
  per_page?: number;
  search?: string;
  status?: UserStatus | "all";
  plan_id?: string;
  node_id?: string;
}

export interface BulkActionPayload {
  user_ids: string[];
  action: "suspend" | "unsuspend" | "change_plan" | "delete";
  plan_id?: string;
  reason?: string;
}

export interface ImpersonateResponse {
  token: string;
  expires_in_seconds: number;
  redirect_url: string;
}
