import React, { useState } from "react";
import { useSession, saveLocalePreference } from "@builtpanel/auth";
import { LanguageSwitcher, useTranslation } from "@builtpanel/i18n";
import { ThemeToggle } from "./ThemeToggle";
import {
  Menu,
  X,
  Server,
  LayoutDashboard,
  Globe,
  Users,
  Settings,
  LogOut,
} from "lucide-react";

export interface NavItem {
  id: string;
  labelKey: string;
  icon: React.ReactNode;
  href: string;
  permission?: string;
  feature?: string;
}

export interface AppShellProps {
  title: string;
  navItems?: NavItem[];
  headerActions?: React.ReactNode;
  children: React.ReactNode;
}

export function AppShell({ title, navItems, headerActions, children }: AppShellProps) {
  const { data: session, logout } = useSession();
  const { t } = useTranslation();
  const [sidebarOpen, setSidebarOpen] = useState(false);

  const defaultNavItems: NavItem[] = [
    {
      id: "dashboard",
      labelKey: "common.dashboard",
      icon: <LayoutDashboard className="h-4 w-4" />,
      href: "/",
      permission: "dashboard.read",
    },
    {
      id: "domains",
      labelKey: "common.domains",
      icon: <Globe className="h-4 w-4" />,
      href: "/domains",
      permission: "domains.read",
      feature: "domains",
    },
    {
      id: "users",
      labelKey: "common.users",
      icon: <Users className="h-4 w-4" />,
      href: "/users",
      permission: "users.read",
    },
    {
      id: "settings",
      labelKey: "common.settings",
      icon: <Settings className="h-4 w-4" />,
      href: "/settings",
      permission: "settings.read",
    },
  ];

  const items = navItems || defaultNavItems;

  return (
    <div className="min-h-screen bg-background text-foreground font-sans">
      {session?.branding?.customCss && (
        <style id="builtpanel-live-custom-css" dangerouslySetInnerHTML={{ __html: session.branding.customCss }} />
      )}
      {/* Mobile Top Navigation */}
      <div className="flex items-center justify-between border-b px-4 py-3 md:hidden">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="rounded p-1 text-muted-foreground hover:bg-muted"
            aria-label="Toggle Menu"
          >
            {sidebarOpen ? <X className="h-5 w-5" /> : <Menu className="h-5 w-5" />}
          </button>
          <div className="flex items-center gap-2 font-bold text-lg text-primary">
            {session?.branding?.logoUrl ? (
              <img src={session.branding.logoUrl} alt="Logo" className="h-5 w-auto object-contain" />
            ) : (
              <Server className="h-5 w-5 text-primary" />
            )}
            <span data-testid="mobile-brand-title">{session?.branding?.companyName || title}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {headerActions}
          <LanguageSwitcher onChange={(l) => void saveLocalePreference(l)} />
          <ThemeToggle />
        </div>
      </div>

      <div className="flex">
        {/* Sidebar */}
        <aside
          className={`fixed inset-y-0 left-0 z-40 w-64 transform border-r bg-card p-4 transition-transform duration-200 ease-in-out md:static md:translate-x-0 ${
            sidebarOpen ? "translate-x-0" : "-translate-x-0 hidden md:block"
          }`}
        >
          <div className="hidden md:flex items-center gap-2 px-2 py-4 border-b">
            {session?.branding?.logoUrl ? (
              <img src={session.branding.logoUrl} alt="Logo" className="h-6 w-auto object-contain" />
            ) : (
              <Server className="h-6 w-6 text-primary" />
            )}
            <span className="text-xl font-bold tracking-tight text-primary" data-testid="app-brand-title">
              {session?.branding?.companyName || title}
            </span>
          </div>

          <div className="py-4">
            <nav className="space-y-1">
              {items.map((item) => (
                <a
                  key={item.id}
                  href={item.href}
                  className="flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
                >
                  {item.icon}
                  <span>{t(item.labelKey)}</span>
                </a>
              ))}
            </nav>
          </div>

          {session?.user && (
            <div className="absolute bottom-4 left-4 right-4 border-t pt-4">
              <div className="flex items-center justify-between px-2 py-1">
                <div className="flex items-center gap-3 overflow-hidden">
                  <div className="h-8 w-8 rounded-full bg-primary/20 flex items-center justify-center text-primary font-bold text-xs shrink-0">
                    {session.user.username.slice(0, 2).toUpperCase()}
                  </div>
                  <div className="overflow-hidden">
                    <p className="text-sm font-medium truncate">{session.user.name}</p>
                    <p className="text-xs text-muted-foreground truncate">{session.user.email}</p>
                  </div>
                </div>
                <button
                  onClick={() => logout()}
                  className="rounded p-1.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive transition-colors shrink-0"
                  aria-label={t("common.logout")}
                  title={t("common.logout")}
                >
                  <LogOut className="h-4 w-4" />
                </button>
              </div>
            </div>
          )}
        </aside>

        {/* Main Content Area */}
        <div className="flex-1 flex flex-col min-h-screen">
          <header className="hidden md:flex h-16 items-center justify-between border-b px-6 bg-card">
            <div>
              <h2 className="text-sm font-medium text-muted-foreground">
                {t("common.welcome")}, <span className="font-semibold text-foreground">{session?.user?.name || "User"}</span>
              </h2>
            </div>
            <div className="flex items-center gap-3">
              {headerActions}
              <LanguageSwitcher onChange={(l) => void saveLocalePreference(l)} />
              <ThemeToggle />
              <button
                onClick={() => logout()}
                className="inline-flex items-center gap-1.5 rounded-md border border-input bg-background px-2.5 py-1.5 text-xs font-medium shadow-sm transition-colors hover:bg-destructive/10 hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={t("common.logout")}
                title={t("common.logout")}
              >
                <LogOut className="h-3.5 w-3.5" />
                <span className="hidden lg:inline">{t("common.logout")}</span>
              </button>
            </div>
          </header>

          <main className="flex-1 p-6 md:p-8">
            {children}
          </main>
        </div>
      </div>
    </div>
  );
}
