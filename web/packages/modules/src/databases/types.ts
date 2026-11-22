export interface DatabaseItem {
  id: string;
  name: string;
  raw_name: string;
  prefix: string;
  /** `mysql` (MariaDB) or `postgres`. */
  engine: string;
  /** `pending`, `ready`, `failed` or `deleting`. */
  status: string;
  size_bytes: number;
  size_formatted: string;
  assigned_users: string[];
}

export interface DatabaseGrant {
  db_id: string;
  db_name: string;
  /** `read_only`, `read_write` or `all`. */
  access: string;
}

export interface DatabaseUserItem {
  id: string;
  username: string;
  raw_username: string;
  prefix: string;
  engine: string;
  status: string;
  /** Addresses (CIDR) it may connect from besides its own network. */
  remote_hosts: string[];
  grants: DatabaseGrant[];
}

export interface DatabaseLimits {
  /** `-1`: no limit. */
  max_databases: number;
  used_databases: number;
  max_db_users: number;
  used_db_users: number;
  used_db_size_bytes: number;
}

export interface ClientDatabasesResponse {
  databases: DatabaseItem[];
  users: DatabaseUserItem[];
  limits: DatabaseLimits;
  /** What every database and user name starts with (`alice_`). */
  prefix: string;
}

export interface AdminDbNodeDistribution {
  node_id: string;
  node_name: string;
  database_count: number;
  total_size_bytes: number;
  total_size_formatted: string;
  db_version: string;
  status: "healthy" | "warning" | "offline";
}

export interface AdminLargestDatabaseItem {
  id: string;
  name: string;
  tenant_id: string;
  tenant_username: string;
  node_id: string;
  node_name: string;
  size_bytes: number;
  size_formatted: string;
  tables_count: number;
  charset: string;
  updated_at: string;
}

export interface AdminDbOverviewResponse {
  summary: {
    total_databases: number;
    total_size_bytes: number;
    total_size_formatted: string;
    total_users: number;
    active_nodes: number;
  };
  node_distributions: AdminDbNodeDistribution[];
  largest_databases: AdminLargestDatabaseItem[];
}
