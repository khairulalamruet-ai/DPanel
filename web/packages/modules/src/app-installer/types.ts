/** One software of the marketplace as `GET /api/v1/admin/marketplace` lists it. */
export interface AdminSoftware {
  slug: string;
  name: string;
  category: string;
  category_name: string;
  description: string;
  license: string;
  homepage: string;
  /** `container` (can be offered), `builtin` (the panel has it) or `unavailable`. */
  kind: "container" | "builtin" | "unavailable";
  version: string;
  provided_by: string;
  reason: string;
  memory_mb: number;
  /** Customers may install it. */
  offered: boolean;
}
