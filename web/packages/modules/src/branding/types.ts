export interface BrandingSettings {
  companyName: string;
  logoUrl: string;
  faviconUrl: string;
  accentColor: string;
  loginTitle: string;
  loginSubtitle: string;
  loginFooterText: string;
  nameservers: string[];
  customCss: string;
}

export const DEFAULT_BRANDING: BrandingSettings = {
  companyName: "Built Panel",
  logoUrl: "",
  faviconUrl: "",
  accentColor: "#3b82f6",
  loginTitle: "Built Panel",
  loginSubtitle: "Sign in to control your web hosting infrastructure",
  loginFooterText: "Built Panel Cloud Infrastructure & Management",
  nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
  customCss: "",
};

/**
 * Sanitizes user-provided custom CSS to prevent XSS / script injection attacks.
 * Rejects HTML tags, javascript:/vbscript: pseudo-protocols, IE expressions/behaviors,
 * and dangerous @import directives.
 */
export function sanitizeCustomCss(rawCss: string): string {
  if (!rawCss) return "";

  let sanitized = rawCss;

  // 1. Strip script, style, iframe, object, embed tags and their contents
  sanitized = sanitized.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, "");
  sanitized = sanitized.replace(/<style\b[^<]*(?:(?!<\/style>)<[^<]*)*<\/style>/gi, "");
  sanitized = sanitized.replace(/<iframe\b[^<]*(?:(?!<\/iframe>)<[^<]*)*<\/iframe>/gi, "");

  // 2. Strip any remaining HTML tags (opening, closing, or self-closing)
  sanitized = sanitized.replace(/<\/?[a-z][a-z0-9]*[^<>]*>/gi, "");

  // 3. Block javascript: & vbscript: pseudo-protocols
  sanitized = sanitized.replace(/javascript\s*:/gi, "blocked-js:");
  sanitized = sanitized.replace(/vbscript\s*:/gi, "blocked-vbscript:");

  // 4. Block CSS expressions (IE)
  sanitized = sanitized.replace(/expression\s*\([^)]*\)/gi, "none");

  // 5. Block HTC behaviors & -moz-binding
  sanitized = sanitized.replace(/behavior\s*:[^;}]*/gi, "behavior: none");
  sanitized = sanitized.replace(/-moz-binding\s*:[^;}]*/gi, "-moz-binding: none");

  // 6. Block external @import to prevent data exfiltration and unverified remote resource loading
  sanitized = sanitized.replace(/@import\s+[^;]+;/gi, "/* @import blocked for security */");

  return sanitized;
}
