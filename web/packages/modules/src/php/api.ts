import { apiFetch } from "@builtpanel/api";
import type { PhpConfigResponse, PhpIniSettings } from "./types";

interface PhpInfo {
  account_version: string;
  applied: boolean;
  available: string[];
  domains: { id: string; fqdn: string; php_version?: string | null; effective_version: string }[];
}

interface PhpSettingsView {
  options: { key: string; kind: string; default: string; choices: string[] }[];
  /** What the account chose: option to text (flags are `On` or `Off`). */
  values: Record<string, string | number | boolean> | null;
  plan_limits: { memory_limit_mb: number; upload_max_mb: number; max_execution_time: number };
}

const toBool = (v: unknown, fallback: boolean): boolean =>
  v === undefined || v === null ? fallback : typeof v === "boolean" ? v : String(v).toLowerCase() === "on" || v === "1" || v === "true";

function iniFrom(view: PhpSettingsView, useValues: boolean): PhpIniSettings {
  const def = (key: string) => view.options.find((o) => o.key === key)?.default ?? "";
  const val = (key: string) => (useValues ? view.values?.[key] : undefined);
  const text = (key: string) => String(val(key) ?? def(key));
  const num = (key: string) => Number(val(key) ?? def(key)) || 0;
  return {
    memory_limit: text("memory_limit"),
    upload_max_filesize: text("upload_max_filesize"),
    post_max_size: text("post_max_size"),
    max_execution_time: num("max_execution_time"),
    max_input_time: num("max_input_time"),
    display_errors: toBool(val("display_errors"), toBool(def("display_errors"), false)),
    date_timezone: text("date.timezone") || "UTC",
  };
}

export async function loadPhp(): Promise<PhpConfigResponse> {
  const [info, settings] = await Promise.all([
    apiFetch<PhpInfo>("/api/v1/client/php"),
    apiFetch<PhpSettingsView>("/api/v1/client/php/settings"),
  ]);
  return {
    default_version: info.account_version,
    supported_versions: info.available.map((version) => ({
      version,
      status: "active",
      is_default: version === info.account_version,
      socket_path: "",
    })),
    domain_configs: info.domains.map((d) => ({
      domain_id: d.id,
      domain_name: d.fqdn,
      php_version: d.effective_version,
      is_custom: !!d.php_version,
    })),
    ini_settings: iniFrom(settings, true),
    plan_defaults: iniFrom(settings, false),
    plan_limits: {
      max_memory_limit: `${settings.plan_limits.memory_limit_mb}M`,
      max_execution_time: settings.plan_limits.max_execution_time,
      max_upload_filesize: `${settings.plan_limits.upload_max_mb}M`,
    },
  };
}

/** The account's PHP version: what its domains use unless they ask for another. */
export async function setAccountVersion(version: string): Promise<void> {
  await apiFetch("/api/v1/client/php", { method: "PUT", body: JSON.stringify({ version }) });
}

/** A domain's own PHP version (`null` goes back to the account's). */
export async function setDomainVersion(domainId: string, version: string | null): Promise<void> {
  await apiFetch(`/api/v1/client/domains/${encodeURIComponent(domainId)}/php`, {
    method: "PUT",
    body: JSON.stringify({ version }),
  });
}

/** Replaces every option the account chose; what is left out goes back to its default. */
export async function saveIni(s: PhpIniSettings): Promise<void> {
  await apiFetch("/api/v1/client/php/settings", {
    method: "PUT",
    body: JSON.stringify({
      values: {
        memory_limit: s.memory_limit,
        upload_max_filesize: s.upload_max_filesize,
        post_max_size: s.post_max_size,
        max_execution_time: s.max_execution_time,
        max_input_time: s.max_input_time,
        display_errors: s.display_errors,
        "date.timezone": s.date_timezone,
      },
    }),
  });
}

export async function resetIni(): Promise<void> {
  await apiFetch("/api/v1/client/php/settings", { method: "PUT", body: JSON.stringify({ values: {} }) });
}
