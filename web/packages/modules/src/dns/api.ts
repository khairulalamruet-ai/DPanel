import { apiFetch } from "@builtpanel/api";
import type {
  BindImportResponse,
  CreateDnsRecordPayload,
  DnsRecord,
  DnsRecordType,
  DnsZone,
  UpdateDnsRecordPayload,
} from "./types";

/** A record as the backend returns it (`RecordView`). */
interface RecordView {
  id: string;
  name: string;
  fqdn: string;
  type: string;
  content: string;
  ttl: number;
  priority?: number | null;
}

interface ZoneInfo {
  name: string;
  /** `pending` (changes not on the server yet), `active` or `error`. */
  status: string;
  last_error?: string | null;
}

interface DomainView {
  id: string;
  fqdn: string;
  unicode: string;
  tenant_id: string;
  created_at: string;
}

const D = "/api/v1/client/domains";
const enc = encodeURIComponent;

/** `weight port target` of an SRV record, `flags tag "value"` of a CAA one. */
export function toRecord(zoneId: string, r: RecordView): DnsRecord {
  const base: DnsRecord = {
    id: r.id,
    zone_id: zoneId,
    name: r.name,
    type: r.type as DnsRecordType,
    content: r.content,
    ttl: r.ttl,
    priority: r.priority ?? null,
    created_at: "",
    updated_at: "",
  };
  if (r.type === "SRV") {
    const m = r.content.match(/^(\d+)\s+(\d+)\s+(.+)$/);
    if (m) return { ...base, weight: Number(m[1]), port: Number(m[2]), content: m[3] };
  }
  if (r.type === "CAA") {
    const m = r.content.match(/^(\d+)\s+(\w+)\s+"?(.*?)"?$/);
    if (m) return { ...base, flags: Number(m[1]), tag: m[2], content: m[3] };
  }
  return base;
}

/** The content the backend wants for a record given the fields of the form. */
export function contentOf(p: {
  type?: string;
  content?: string;
  weight?: number | null;
  port?: number | null;
  flags?: number | null;
  tag?: string | null;
}): string | undefined {
  if (p.content === undefined) return undefined;
  if (p.type === "SRV") return `${p.weight ?? 0} ${p.port ?? 0} ${p.content}`;
  if (p.type === "CAA") return `${p.flags ?? 0} ${p.tag ?? "issue"} "${p.content}"`;
  return p.content;
}

export async function listZones(): Promise<DnsZone[]> {
  const [domains, ns] = await Promise.all([
    apiFetch<{ items: DomainView[] }>(D),
    apiFetch<{ nameservers: string[] }>("/api/v1/client/dns/nameservers").catch(() => ({ nameservers: [] as string[] })),
  ]);
  return domains.items.map((d) => ({
    id: d.id,
    domain_id: d.id,
    domain_name: d.unicode || d.fqdn,
    tenant_id: d.tenant_id,
    nameservers: ns.nameservers,
    soa_serial: 0,
    records_count: 0,
    is_custom_ns: false,
    dnssec_enabled: false,
    updated_at: d.created_at,
  }));
}

export async function loadZone(zone: DnsZone): Promise<{ zone: DnsZone; records: DnsRecord[] }> {
  const res = await apiFetch<{ items: RecordView[]; zone: ZoneInfo }>(`${D}/${enc(zone.id)}/dns/records`);
  const records = res.items.map((r) => toRecord(zone.id, r));
  return {
    zone: { ...zone, records_count: records.length, status: res.zone.status, last_error: res.zone.last_error ?? null },
    records,
  };
}

export async function createRecord(zoneId: string, p: CreateDnsRecordPayload): Promise<DnsRecord> {
  const res = await apiFetch<RecordView>(`${D}/${enc(zoneId)}/dns/records`, {
    method: "POST",
    body: JSON.stringify({
      name: p.name || "@",
      type: p.type,
      content: contentOf(p),
      ttl: p.ttl,
      ...(p.type === "MX" || p.type === "SRV" ? { priority: p.priority ?? 0 } : {}),
    }),
  });
  return toRecord(zoneId, res);
}

/** Only the data, TTL and priority of a record can change; name and type are fixed. */
export async function updateRecord(zoneId: string, current: DnsRecord, p: UpdateDnsRecordPayload): Promise<DnsRecord> {
  const type = p.type ?? current.type;
  const res = await apiFetch<RecordView>(`${D}/${enc(zoneId)}/dns/records/${enc(current.id)}`, {
    method: "PATCH",
    body: JSON.stringify({
      content: contentOf({ type, ...p }),
      ttl: p.ttl,
      ...(type === "MX" || type === "SRV" ? { priority: p.priority ?? current.priority ?? 0 } : {}),
    }),
  });
  return toRecord(zoneId, res);
}

export async function deleteRecord(zoneId: string, recordId: string): Promise<void> {
  await apiFetch(`${D}/${enc(zoneId)}/dns/records/${enc(recordId)}`, { method: "DELETE" });
}

export interface Delegation {
  ok: boolean;
  status: string;
  expected: string[];
  found: string[];
  missing: string[];
}

/** Asks the public DNS which name servers the domain uses and compares them with the panel's. */
export async function verifyNameservers(zoneId: string): Promise<Delegation> {
  const res = await apiFetch<{ status: string; expected: string[]; found: string[]; missing: string[] }>(
    `${D}/${enc(zoneId)}/dns/check`,
    { method: "POST" },
  );
  return { ok: res.status === "ok", ...res };
}

// ---- BIND zone files, made and read here (the backend keeps records, not zone files)

const fqdn = (host: string) => (host.endsWith(".") ? host : `${host}.`);

export function toBind(domain: string, records: DnsRecord[], nameservers: string[] = []): string {
  const now = new Date();
  const serial = `${now.getUTCFullYear()}${String(now.getUTCMonth() + 1).padStart(2, "0")}${String(now.getUTCDate()).padStart(2, "0")}01`;
  const lines = [
    `$ORIGIN ${fqdn(domain)}`,
    `; zone ${domain}, exported from the panel`,
    `@\t3600\tIN\tSOA\t${fqdn(nameservers[0] ?? `ns1.${domain}`)} ${fqdn(`hostmaster.${domain}`)} ${serial} 7200 3600 1209600 3600`,
  ];
  for (const r of records) {
    const data =
      r.type === "SRV"
        ? `${r.priority ?? 0} ${r.weight ?? 0} ${r.port ?? 0} ${fqdn(r.content)}`
        : r.type === "MX"
          ? `${r.priority ?? 0} ${fqdn(r.content)}`
          : r.type === "CNAME" || r.type === "NS"
            ? fqdn(r.content)
            : r.type === "TXT"
              ? `"${r.content.replace(/"/g, '\\"')}"`
              : r.type === "CAA"
                ? `${r.flags ?? 0} ${r.tag ?? "issue"} "${r.content}"`
                : r.content;
    lines.push(`${r.name}\t${r.ttl}\tIN\t${r.type}\t${data}`);
  }
  return lines.join("\n") + "\n";
}

const SUPPORTED = new Set(["A", "AAAA", "CNAME", "MX", "TXT", "SRV", "CAA", "NS"]);
const stripDot = (host: string) => host.replace(/\.$/, "");

/** Reads the records out of a zone file; SOA and what the panel does not support are skipped with a warning. */
export function parseBind(text: string, domain: string): { records: CreateDnsRecordPayload[]; warnings: string[] } {
  const records: CreateDnsRecordPayload[] = [];
  const warnings: string[] = [];
  let origin = stripDot(domain);
  let defaultTtl = 3600;
  let lastName = "@";
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/(^|\s);.*$/, "").trim();
    if (!line) continue;
    if (line.startsWith("$ORIGIN")) {
      origin = stripDot(line.split(/\s+/)[1] ?? origin);
      continue;
    }
    if (line.startsWith("$TTL")) {
      defaultTtl = Number(line.split(/\s+/)[1]) || defaultTtl;
      continue;
    }
    if (line.startsWith("$")) continue;
    const tokens = line.match(/"[^"]*"|\S+/g) ?? [];
    let i = 0;
    // A line that starts with white space repeats the previous name.
    let name = /^\s/.test(raw) ? lastName : tokens[i++];
    let ttl = defaultTtl;
    while (i < tokens.length && (/^\d+$/.test(tokens[i]) || tokens[i].toUpperCase() === "IN")) {
      if (/^\d+$/.test(tokens[i])) ttl = Number(tokens[i]);
      i++;
    }
    const type = (tokens[i++] ?? "").toUpperCase();
    const rdata = tokens.slice(i);
    if (name === origin || name === `${origin}.`) name = "@";
    else if (name.endsWith(`.${origin}.`)) name = name.slice(0, -(origin.length + 2));
    else if (name.endsWith(".") ) {
      warnings.push(`${name} is outside the zone; skipped`);
      continue;
    }
    lastName = name;
    if (type === "SOA") continue;
    if (!SUPPORTED.has(type)) {
      warnings.push(`${type} records are not supported; skipped ${name}`);
      continue;
    }
    const host = (h: string) => stripDot(h === "@" ? origin : h);
    const unquote = (t: string) => t.replace(/^"|"$/g, "");
    if (type === "MX" && rdata.length >= 2) {
      records.push({ name, type, ttl, priority: Number(rdata[0]), content: host(rdata[1]) });
    } else if (type === "SRV" && rdata.length >= 4) {
      records.push({ name, type, ttl, priority: Number(rdata[0]), weight: Number(rdata[1]), port: Number(rdata[2]), content: host(rdata[3]) });
    } else if (type === "CAA" && rdata.length >= 3) {
      records.push({ name, type, ttl, flags: Number(rdata[0]), tag: rdata[1], content: unquote(rdata[2]) });
    } else if (type === "TXT") {
      records.push({ name, type, ttl, content: rdata.map(unquote).join("") });
    } else if (type === "CNAME" || type === "NS") {
      records.push({ name, type, ttl, content: host(rdata[0] ?? "") });
    } else if (rdata.length >= 1) {
      records.push({ name, type: type as DnsRecordType, ttl, content: rdata[0] });
    }
  }
  return { records, warnings };
}

export async function importBind(
  zoneId: string,
  domain: string,
  text: string,
  overwrite: boolean,
): Promise<BindImportResponse> {
  const { records, warnings } = parseBind(text, domain);
  let existing: DnsRecord[] = [];
  if (overwrite) {
    const zone = await apiFetch<{ items: RecordView[] }>(`${D}/${enc(zoneId)}/dns/records`);
    existing = zone.items.map((r) => toRecord(zoneId, r));
  }
  const created: DnsRecord[] = [];
  let skipped = warnings.length;
  for (const rec of records) {
    try {
      if (overwrite) {
        // The apex name servers belong to the panel.
        for (const old of existing.filter((e) => e.name === rec.name && e.type === rec.type && !(e.type === "NS" && e.name === "@"))) {
          await deleteRecord(zoneId, old.id);
        }
      }
      created.push(await createRecord(zoneId, rec));
    } catch (err: any) {
      skipped++;
      warnings.push(`${rec.name} ${rec.type}: ${err?.message ?? "refused"}`);
    }
  }
  return { imported_count: created.length, skipped_count: skipped, warnings, records: created };
}
