import { apiFetch } from "@builtpanel/api";

const IMAGES = "/api/v1/admin/container-images";
const SMTP = "/api/v1/admin/notifications/smtp";

export interface ContainerImage {
  id: string;
  name: string;
  reference?: string | null;
  note: string;
  default_port: number;
  default_memory_mb: number;
  enabled: boolean;
}
export interface ImageInput {
  name: string;
  reference?: string | null;
  note?: string | null;
  default_port: number;
  default_memory_mb?: number | null;
  enabled?: boolean | null;
}

export interface SmtpSettings {
  enabled: boolean;
  host: string;
  port: number;
  security: string;
  username?: string | null;
  from_address: string;
  from_name?: string | null;
  has_password: boolean;
}
export interface SmtpInput {
  enabled: boolean;
  host: string;
  port: number;
  security: string;
  username?: string | null;
  password?: string | null;
  from_address: string;
  from_name?: string | null;
}

export const listImages = () => apiFetch<ContainerImage[]>(IMAGES);
export const addImage = (i: ImageInput) =>
  apiFetch<ContainerImage>(IMAGES, { method: "POST", body: JSON.stringify(i) });
export const changeImage = (id: string, i: ImageInput) =>
  apiFetch<ContainerImage>(`${IMAGES}/${encodeURIComponent(id)}`, { method: "PUT", body: JSON.stringify(i) });
export const removeImage = (id: string) =>
  apiFetch<void>(`${IMAGES}/${encodeURIComponent(id)}`, { method: "DELETE" });

export const getSmtp = () => apiFetch<SmtpSettings | null>(SMTP);
export const putSmtp = (s: SmtpInput) => apiFetch<void>(SMTP, { method: "PUT", body: JSON.stringify(s) });
export const testSmtp = (to: string) =>
  apiFetch<void>(`${SMTP}/test`, { method: "POST", body: JSON.stringify({ to }) });
