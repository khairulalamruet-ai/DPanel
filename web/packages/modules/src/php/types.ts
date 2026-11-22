export type PhpStatus = "active" | "deprecated" | "eol";

export interface PhpVersion {
  version: string; // e.g. "8.3", "8.2", "8.1", "8.4"
  status: PhpStatus;
  is_default: boolean;
  socket_path: string;
}

export interface PhpDomainConfig {
  domain_id: string;
  domain_name: string;
  php_version: string;
  is_custom: boolean;
}

export interface PhpIniSettings {
  memory_limit: string;
  upload_max_filesize: string;
  post_max_size: string;
  max_execution_time: number;
  max_input_time: number;
  display_errors: boolean;
  date_timezone: string;
}

export interface PhpPlanLimits {
  max_memory_limit: string; // e.g. "512M"
  max_execution_time: number; // e.g. 300
  max_upload_filesize: string; // e.g. "128M"
}

export interface PhpConfigResponse {
  default_version: string;
  supported_versions: PhpVersion[];
  domain_configs: PhpDomainConfig[];
  ini_settings: PhpIniSettings;
  plan_defaults: PhpIniSettings;
  plan_limits: PhpPlanLimits;
}
