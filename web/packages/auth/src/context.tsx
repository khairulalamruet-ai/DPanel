import React, { createContext, useContext, useEffect, useState } from "react";
import { setCsrfToken, getCsrfToken, apiFetch, api } from "@builtpanel/api";
import { BootstrapData } from "./types";

interface SessionContextType {
  data: BootstrapData | null;
  /** The look of the panel before anybody is signed in (the sign-in page). */
  publicBranding: BootstrapData["branding"] | null;
  isLoading: boolean;
  error: Error | null;
  hasPermission: (perm: string) => boolean;
  hasFeature: (feature: string) => boolean;
  hasRole: (role: string) => boolean;
  refetch: () => Promise<void>;
  logout: () => Promise<void>;
  setSessionData: (data: BootstrapData | null) => void;
}


/**
 * Reads what the backend answers at start-up: the administrators' or customers' bootstrap
 * (user, roles, permissions, modules with menu data, plan features and limits, branding,
 * locale and the CSRF token), or the plain `GET /api/v1/auth/me`.
 */
export function normalizeBootstrap(res: any): BootstrapData {
  const user = res.user ?? res;
  const isAdmin = user.kind === "admin";
  const branding = res.branding ?? {};
  const modules: string[] | undefined = Array.isArray(res.modules)
    ? res.modules.map((m: any) => (typeof m === "string" ? m : m.name))
    : undefined;
  const features: string[] = Array.isArray(res.features) ? res.features : [];
  return {
    user: {
      id: user.id,
      username: user.username,
      email: user.email,
      name: user.name ?? user.username,
      kind: user.kind,
    },
    roles: res.roles || [],
    permissions: res.permissions || [],
    modules,
    // Administrators have no plan: every feature is theirs.
    features: isAdmin ? ["all"] : features,
    limits: res.limits ?? {},
    branding: {
      companyName: branding.companyName ?? branding.panel_name ?? "Built Panel",
      logoUrl: branding.logoUrl || branding.logo_url || undefined,
      faviconUrl: branding.faviconUrl || branding.favicon_url || undefined,
      accentColor: branding.accentColor || branding.accent_color || undefined,
      loginTitle: branding.loginTitle || branding.login_title || undefined,
      loginSubtitle: branding.loginSubtitle || branding.login_subtitle || undefined,
      loginFooterText: branding.loginFooterText,
      nameservers: branding.nameservers,
      customCss: branding.customCss,
      maintenanceNotice: branding.maintenance_notice || undefined,
    },
    locale: res.locale || "en",
    csrfToken: res.csrfToken ?? res.csrf_token,
    mfa_enabled: res.mfa_enabled ?? res.mfa?.enabled,
  };
}

/** `#rrggbb` as the "H S% L%" triple the theme variables use, and whether dark text reads better on it. */
export function hexToHsl(hex: string): { hsl: string; darkText: boolean } | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  const r = ((n >> 16) & 255) / 255;
  const g = ((n >> 8) & 255) / 255;
  const b = (n & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
  }
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return {
    hsl: `${Math.round(h)} ${Math.round(s * 100)}% ${Math.round(l * 100)}%`,
    darkText: luminance > 0.6,
  };
}

/** Puts the operator's accent colour and tab icon on the page. */
function applyBranding(branding: { accentColor?: string; faviconUrl?: string } | undefined) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const accent = branding?.accentColor ? hexToHsl(branding.accentColor) : null;
  if (accent) {
    root.style.setProperty("--primary", accent.hsl);
    root.style.setProperty("--ring", accent.hsl);
    root.style.setProperty("--primary-foreground", accent.darkText ? "0 0% 9%" : "0 0% 100%");
  } else {
    for (const name of ["--primary", "--ring", "--primary-foreground"]) root.style.removeProperty(name);
  }
  if (branding?.faviconUrl) {
    let link = document.querySelector<HTMLLinkElement>("link[rel*='icon']");
    if (!link) {
      link = document.createElement("link");
      link.rel = "icon";
      document.head.appendChild(link);
    }
    link.href = branding.faviconUrl;
  }
}

const SessionContext = createContext<SessionContextType | undefined>(undefined);

export interface SessionProviderProps {
  children: React.ReactNode;
  bootstrapPath?: string;
}

export function SessionProvider({ children, bootstrapPath = "/api/v1/auth/me" }: SessionProviderProps) {
  const [data, setData] = useState<BootstrapData | null>(null);
  const [publicBranding, setPublicBranding] = useState<BootstrapData["branding"] | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  const fetchSession = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<any>(bootstrapPath);
      const normalized = normalizeBootstrap(res);
      setData(normalized);
      if (normalized.csrfToken || res.csrf_token) {
        setCsrfToken(normalized.csrfToken || res.csrf_token);
      }
    } catch (err: any) {
      setError(err instanceof Error ? err : new Error(String(err)));
      // Nobody is signed in: the sign-in page still shows the operator's name, logo and colour.
      try {
        const look = await apiFetch<any>("/api/v1/public/branding");
        setPublicBranding(normalizeBootstrap({ user: {}, branding: look }).branding);
      } catch {
        // The plain look is used.
      }
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchSession();
  }, [bootstrapPath]);

  useEffect(() => {
    const look = data?.branding ?? publicBranding ?? undefined;
    applyBranding(look);
  }, [data?.branding?.accentColor, data?.branding?.faviconUrl, publicBranding]);

  const hasPermission = (perm: string): boolean => {
    if (!data) return false;
    if (data.roles.includes("super_admin") || data.permissions.includes("*")) return true;
    if (data.permissions.includes(perm)) return true;
    return data.permissions.some((p) => {
      if (p.endsWith(".*")) {
        const prefix = p.slice(0, -2);
        return perm === prefix || perm.startsWith(`${prefix}.`);
      }
      return false;
    });
  };

  const hasFeature = (feature: string): boolean => {
    if (!data) return false;
    if (data.features?.includes("all")) return true;
    return data.features?.includes(feature) ?? false;
  };

  const hasRole = (role: string): boolean => {
    if (!data) return false;
    return data.roles.includes(role);
  };

  const logout = async () => {
    try {
      await api.POST("/api/v1/auth/logout", {
        params: { header: { "X-CSRF-Token": getCsrfToken() || "" } },
      });
    } catch {
      // Ignore network errors on logout
    } finally {
      setData(null);
      setCsrfToken(null);
    }
  };

  return (
    <SessionContext.Provider
      value={{
        data,
        publicBranding,
        isLoading,
        error,
        hasPermission,
        hasFeature,
        hasRole,
        refetch: fetchSession,
        logout,
        setSessionData: setData,
      }}
    >
      {children}
    </SessionContext.Provider>
  );
}

/** The session when there is one (pages that can also be shown without it). */
export function useOptionalSession() {
  return useContext(SessionContext);
}

export function useSession() {
  const context = useContext(SessionContext);
  if (!context) {
    throw new Error("useSession must be used within a SessionProvider");
  }
  return context;
}
