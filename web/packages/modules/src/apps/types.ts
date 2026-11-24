export type AppCategory = "cms" | "framework" | "e-commerce" | "static";

export interface CatalogApp {
  id: string;
  name: string;
  category: AppCategory;
  version: string;
  description: string;
  description_bn?: string;
  icon: string;
  requires_db: boolean;
  requires_php: boolean;
  min_php_version?: string;
  documentation_url?: string;
}

export type InstalledAppStatus = "running" | "maintenance" | "installing" | "error";

export interface InstalledApp {
  id: string;
  catalog_app_id: string;
  name: string;
  domain: string;
  path: string;
  url: string;
  admin_url?: string;
  version: string;
  latest_version: string;
  has_update: boolean;
  status: InstalledAppStatus;
  db_name?: string;
  db_user?: string;
  installed_at: string;
  auto_update: boolean;
}

export interface WordPressPlugin {
  slug: string;
  name: string;
  version: string;
  latest_version: string;
  has_update: boolean;
  active: boolean;
  author: string;
}

export interface WordPressTheme {
  slug: string;
  name: string;
  version: string;
  active: boolean;
}

export interface WordPressScan {
  checksums_ok: boolean;
  config_mode: string;
  debug: boolean;
  extra: string[];
  file_edit_allowed: boolean;
  missing: string[];
  modified: string[];
  world_writable: number;
  xmlrpc_disabled: boolean;
}

export interface WordPressDetails {
  app_id: string;
  core_version: string;
  latest_core_version: string;
  maintenance_mode: boolean;
  xmlrpc_disabled: boolean;
  debug_mode: boolean;
  /** What the last security scan found. */
  scan?: WordPressScan | null;
  plugins: WordPressPlugin[];
  themes: WordPressTheme[];
}

export interface InstallAppPayload {
  catalog_app_id: string;
  domain: string;
  path: string;
  protocol: "https" | "http";
  site_name: string;
  admin_username: string;
  admin_email: string;
  admin_password: string;
  db_name?: string;
  db_user?: string;
  db_password?: string;
  auto_update?: boolean;
}
