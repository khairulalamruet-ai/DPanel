import { apiFetch } from "@builtpanel/api";
import { initialPageTemplates } from "./templates";
import type { PageTemplateItem, PageTemplateType } from "./types";

/** The names the backend gives the four pages. */
const KIND: Record<PageTemplateType, string> = {
  suspended_domain: "suspended_domain",
  suspended_user: "suspended_account",
  error_404: "not_found",
  error_502: "bad_gateway",
};

interface PageView {
  kind: string;
  html: string;
  is_custom: boolean;
}

interface PageList {
  items: PageView[];
  max_bytes: number;
}

const B = "/api/v1/admin/error-pages";

function merge(list: PageList): PageTemplateItem[] {
  return initialPageTemplates.map((meta) => {
    const page = list.items.find((p) => p.kind === KIND[meta.type]);
    return {
      ...meta,
      content: page?.html ?? meta.defaultContent,
      isCustomized: page?.is_custom ?? false,
      updatedAt: new Date().toISOString(),
    };
  });
}

export async function listPages(): Promise<{ items: PageTemplateItem[]; maxBytes: number }> {
  const res = await apiFetch<PageList>(B);
  return { items: merge(res), maxBytes: res.max_bytes };
}

/** Replaces a page; the answer is the whole list again. */
export async function savePage(type: PageTemplateType, html: string): Promise<PageTemplateItem> {
  const res = await apiFetch<PageList>(`${B}/${KIND[type]}`, { method: "PUT", body: JSON.stringify({ html }) });
  return merge(res).find((p) => p.type === type) as PageTemplateItem;
}

/** Goes back to the installation's own page. */
export async function resetPage(type: PageTemplateType): Promise<PageTemplateItem> {
  await apiFetch(`${B}/${KIND[type]}`, { method: "DELETE" });
  const { items } = await listPages();
  return items.find((p) => p.type === type) as PageTemplateItem;
}
