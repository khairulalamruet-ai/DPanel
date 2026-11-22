import React, { createContext, useContext, useEffect, useState } from "react";
import { setCsrfToken, getCsrfToken, apiFetch, api } from "@builtpanel/api";
import { BootstrapData } from "./types";

interface SessionContextType {
  data: BootstrapData | null;
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
      logoUrl: branding.logoUrl,
      faviconUrl: branding.faviconUrl,
      accentColor: branding.accentColor,
      loginTitle: branding.loginTitle,
      loginSubtitle: branding.loginSubtitle,
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

const SessionContext = createContext<SessionContextType | undefined>(undefined);

export interface SessionProviderProps {
  children: React.ReactNode;
  bootstrapPath?: string;
}

export function SessionProvider({ children, bootstrapPath = "/api/v1/auth/me" }: SessionProviderProps) {
  const [data, setData] = useState<BootstrapData | null>(null);
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
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchSession();
  }, [bootstrapPath]);

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
