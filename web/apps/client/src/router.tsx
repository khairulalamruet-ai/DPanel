import React from "react";
import {
  createRouter,
  createRoute,
  createRootRoute,
  Outlet,
} from "@tanstack/react-router";
import { ThemeProvider, ToastProvider } from "@builtpanel/ui";
import { SessionProvider, useSession, LoginPage, AccountSecurityPage } from "@builtpanel/auth";
import { queryClient, QueryClientProvider } from "@builtpanel/api";
import { AppShell, NotFoundPage, RouteErrorFallback } from "@builtpanel/shell";
import { ClientDashboardPage, ClientDomainsPage, DnsEditorPage, SslPage, PhpManagementPage, ServicesPage, ClientDatabasesPage, FileManagerPage, DiskUsagePage, FtpAccountsPage, ClientBackupsPage, CronJobsPage, LogViewerPage, ClientUsagePage, ClientProcessesPage, ClientWafPage, ClientFirewallPage, AppsPage, ClientEmailPage, ResellerPortalPage, MigrationPage, NotificationBell, NotificationPreferencesPage, CommandPalette, CommandPaletteTrigger } from "@builtpanel/modules";
import { useTranslation } from "@builtpanel/i18n";
import { LayoutDashboard, ShieldCheck, ShieldAlert, Globe, Lock, Database, Folder, Server, Archive, Clock, FileText, Activity, Terminal, Flame, Boxes, Mail, Briefcase, ArrowRightLeft } from "lucide-react";

/** The plan feature a menu entry needs; entries that are not listed are always shown. */
const NAV_FEATURE: Record<string, string> = {
  domains: "domains",
  dns: "dns",
  ssl: "ssl",
  databases: "databases",
  files: "files",
  ftp: "ftp",
  backups: "backups",
  cron: "cron",
  logs: "logs",
};

/** Pages whose backend does not exist yet: they stay out of the menu until it does. */
const NOT_YET = new Set(["firewall", "migration"]);

function AuthenticatedContent() {
  const { data: session, isLoading, hasFeature } = useSession();
  const { t } = useTranslation();
  const [commandPaletteOpen, setCommandPaletteOpen] = React.useState(false);

  const navItems = [
    {
      id: "dashboard",
      labelKey: "common.dashboard",
      icon: <LayoutDashboard className="h-4 w-4" />,
      href: "/",
    },
    {
      id: "domains",
      labelKey: "domains.nav",
      icon: <Globe className="h-4 w-4" />,
      href: "/domains",
    },
    {
      id: "dns",
      labelKey: "dns.nav",
      icon: <Globe className="h-4 w-4" />,
      href: "/dns",
    },
    {
      id: "ssl",
      labelKey: "ssl.nav",
      icon: <Lock className="h-4 w-4" />,
      href: "/ssl",
    },
    {
      id: "databases",
      labelKey: "databases.nav",
      icon: <Database className="h-4 w-4" />,
      href: "/databases",
    },
    {
      id: "files",
      labelKey: "files.nav",
      icon: <Folder className="h-4 w-4" />,
      href: "/files",
    },
    {
      id: "ftp",
      labelKey: "ftp.nav",
      icon: <Server className="h-4 w-4" />,
      href: "/ftp",
    },
    {
      id: "backups",
      labelKey: "backups.nav",
      icon: <Archive className="h-4 w-4" />,
      href: "/backups",
    },
    {
      id: "cron",
      labelKey: "cron.nav",
      icon: <Clock className="h-4 w-4" />,
      href: "/cron",
    },
    {
      id: "logs",
      labelKey: "logs.nav",
      icon: <FileText className="h-4 w-4" />,
      href: "/logs",
    },
    {
      id: "usage",
      labelKey: "usage.nav",
      icon: <Activity className="h-4 w-4" />,
      href: "/usage",
    },
    {
      id: "processes",
      labelKey: "processes.nav",
      icon: <Terminal className="h-4 w-4" />,
      href: "/processes",
    },
    {
      id: "waf",
      labelKey: "waf.nav",
      icon: <ShieldAlert className="h-4 w-4" />,
      href: "/waf",
    },
    {
      id: "firewall",
      labelKey: "firewall.nav",
      icon: <Flame className="h-4 w-4" />,
      href: "/firewall",
    },
    {
      id: "apps",
      labelKey: "apps.nav",
      icon: <Boxes className="h-4 w-4" />,
      href: "/apps",
    },
    {
      id: "email",
      labelKey: "email.nav",
      icon: <Mail className="h-4 w-4" />,
      href: "/email",
    },
    {
      id: "reseller",
      labelKey: "reseller.nav",
      icon: <Briefcase className="h-4 w-4" />,
      href: "/reseller",
    },
    {
      id: "migration",
      labelKey: "migration.nav",
      icon: <ArrowRightLeft className="h-4 w-4" />,
      href: "/migration",
    },
    {
      id: "security",
      labelKey: "security.nav",
      icon: <ShieldCheck className="h-4 w-4" />,
      href: "/security",
    },
  ];

  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-background">
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      </div>
    );
  }

  if (!session?.user) {
    return (
      <LoginPage
        title={t("common.clientTitle")}
        onSuccess={() => router.navigate({ to: "/" })}
      />
    );
  }

  return (
    <>
      <AppShell
        title={t("common.clientTitle")}
        navItems={navItems.filter(
          (item) => !NOT_YET.has(item.id) && (!NAV_FEATURE[item.id] || hasFeature(NAV_FEATURE[item.id])),
        )}
        headerActions={
          <div className="flex items-center gap-2">
            <CommandPaletteTrigger onClick={() => setCommandPaletteOpen(true)} />
            <NotificationBell settingsHref="/settings/notifications" />
          </div>
        }
      >
        <Outlet />
      </AppShell>
      <CommandPalette
        open={commandPaletteOpen}
        onOpenChange={setCommandPaletteOpen}
        onNavigate={(href) => router.navigate({ to: href as any })}
      />
    </>
  );
}

function RootLayout() {
  return (
    <ThemeProvider defaultTheme="system" storageKey="builtpanel-client-theme">
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <SessionProvider bootstrapPath="/api/v1/client/bootstrap">
            <AuthenticatedContent />
          </SessionProvider>
        </ToastProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}

const rootRoute = createRootRoute({
  component: RootLayout,
  errorComponent: RouteErrorFallback,
  notFoundComponent: () => <NotFoundPage onNavigateHome={() => router.navigate({ to: "/" })} />,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: ClientDashboardPage,
});

const domainsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/domains",
  component: ClientDomainsPage,
});

const dnsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dns",
  component: DnsEditorPage,
});

const sslRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/ssl",
  component: SslPage,
});

const phpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/php",
  component: PhpManagementPage,
});

const servicesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/services",
  component: ServicesPage,
});

const databasesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/databases",
  component: ClientDatabasesPage,
});

const filesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/files",
  component: FileManagerPage,
});

const diskUsageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/disk-usage",
  component: DiskUsagePage,
});

const ftpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/ftp",
  component: FtpAccountsPage,
});

const backupsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/backups",
  component: ClientBackupsPage,
});

const cronRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/cron",
  component: CronJobsPage,
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: LogViewerPage,
});

const usageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/usage",
  component: ClientUsagePage,
});

const processesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/processes",
  component: ClientProcessesPage,
});

const wafRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/waf",
  component: ClientWafPage,
});

const firewallRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/firewall",
  component: ClientFirewallPage,
});

const appsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps",
  component: AppsPage,
});

const emailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/email",
  component: ClientEmailPage,
});

const resellerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/reseller",
  component: ResellerPortalPage,
});

const migrationRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/migration",
  component: MigrationPage,
});

const securityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/security",
  component: AccountSecurityPage,
});

const notificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/notifications",
  component: NotificationPreferencesPage,
});

const settingsNotificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/notifications",
  component: NotificationPreferencesPage,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: () => {
    const { t } = useTranslation();
    return (
      <LoginPage
        title={t("common.clientTitle")}
        onSuccess={() => router.navigate({ to: "/" })}
      />
    );
  },
});

const routeTree = rootRoute.addChildren([indexRoute, domainsRoute, dnsRoute, sslRoute, phpRoute, servicesRoute, databasesRoute, filesRoute, diskUsageRoute, ftpRoute, backupsRoute, cronRoute, logsRoute, usageRoute, processesRoute, wafRoute, firewallRoute, appsRoute, emailRoute, resellerRoute, migrationRoute, securityRoute, notificationsRoute, settingsNotificationsRoute, loginRoute]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  defaultErrorComponent: RouteErrorFallback,
  notFoundMode: "root",
});
