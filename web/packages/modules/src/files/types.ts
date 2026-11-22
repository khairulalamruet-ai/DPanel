export type FileItemType = "file" | "directory" | "symlink";

export interface FileItem {
  id: string;
  name: string;
  path: string;
  type: FileItemType;
  size_bytes: number;
  size_formatted: string;
  mime_type?: string;
  extension?: string;
  permissions: string; // e.g. "0644", "0755"
  owner: string;
  group: string;
  updated_at: string;
  in_trash?: boolean;
  original_path?: string;
}

export interface FolderTreeNode {
  path: string;
  name: string;
  has_children: boolean;
  children?: FolderTreeNode[];
}

export interface FileListResponse {
  current_path: string;
  parent_path?: string | null;
  items: FileItem[];
  total_files: number;
  total_dirs: number;
  total_size_bytes: number;
  total_size_formatted: string;
}

export interface TrashListResponse {
  items: FileItem[];
  total_items: number;
}

export interface ReadFileResponse {
  path: string;
  content: string;
  encoding: string;
  size_bytes: number;
  mime_type: string;
}

export interface WriteFilePayload {
  path: string;
  content: string;
}

export interface CreateItemPayload {
  parent_path: string;
  name: string;
  type: "file" | "directory";
}

export interface ChmodPayload {
  path: string;
  mode: string;
}

export interface RenamePayload {
  old_path: string;
  new_name: string;
}

export interface ArchivePayload {
  paths: string[];
  archive_name: string;
  format: "zip" | "tar.gz";
}

export interface ExtractPayload {
  archive_path: string;
  destination_path: string;
}

export interface RestoreTrashPayload {
  item_ids: string[];
}

export interface DirectoryUsageItem {
  path: string;
  name: string;
  size_bytes: number;
  size_formatted: string;
  percentage_of_total: number;
  inodes_count: number;
  files_count: number;
  last_calculated_at: string;
}

export interface DiskUsageResponse {
  disk_used_bytes: number;
  disk_used_formatted: string;
  disk_limit_bytes: number;
  disk_limit_formatted: string;
  disk_usage_pct: number;
  inodes_used: number;
  inodes_limit: number;
  inodes_usage_pct: number;
  last_scanned_at: string;
  /** The measurement stopped at its limit, so the figures are a lower bound. */
  partial?: boolean;
  top_directories: DirectoryUsageItem[];
}
