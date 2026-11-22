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
import {
  DashboardPage,
  UiDemoPage,
  AuditViewerPage,
  DummyModulePage,
  ModuleManagerPage,
  NodesPage,
  AdminIpPoolsPage,
  OperationsPage,
  SettingsPage,
  FeatureSetsPage,
  PlansPage,
  UsersPage,
  UserCreateWizardPage,
  UserDetailPage,
  ClientDashboardPage,
  ClientDomainsPage,
  AdminDomainsPage,
  DnsEditorPage,
  SslPage,
  VhostTemplatesPage,
  PhpManagementPage,
  ServicesPage,
  ErrorPagesPage,
  ClientDatabasesPage,
  AdminDatabasesPage,
  FileManagerPage,
  DiskUsagePage,
  FtpAccountsPage,
  ClientBackupsPage,
  AdminBackupsPage,
  CronJobsPage,
  LogViewerPage,
  AdminSystemLogsPage,
  ClientUsagePage,
  AdminUsagePage,
  ClientProcessesPage,
  AdminProcessesPage,
  AdminServerToolsPage,
  ClientWafPage,
  AdminWafPage,
  MigrationPage,
  LicensePage,
  LaAdminPage,
  NotificationBell,
  NotificationPreferencesPage,
  BrandingPage,
  CommandPalette,
  CommandPaletteTrigger,
  UpdatePage,
  ApiTokensPage,
  SupportReportPage,
  AdminDnsClusterPage,
  AppsPage,
  ClientEmailPage,
  AdminMailQueuePage,
  ClientFirewallPage,
  AdminFirewallPage,
  ResellerPortalPage,
  AdminResellersPage,
  AdministratorsPage,
  AdminSecurityCenterPage,
  AdminSystemToolsPage,
  filterNavItems,
  registeredFrontendModules,
} from "@builtpanel/modules";
import { useTranslation } from "@builtpanel/i18n";

function AuthenticatedContent() {
  const { data: session, isLoading } = useSession();
  const { t } = useTranslation();
  const [commandPaletteOpen, setCommandPaletteOpen] = React.useState(false);

  const navItems = filterNavItems(registeredFrontendModules, session, "admin");

  if (isLoading) {
    return (
      <div className="flex h-screen w-screen items-center justify-center">
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      </div>
    );
  }

  if (!session?.user) {
    return (
      <LoginPage
        title={t("common.adminTitle")}
        onSuccess={() => router.navigate({ to: "/" })}
      />
    );
  }

  return (
    <>
      <AppShell
        title={t("common.adminTitle")}
        navItems={navItems}
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
    <QueryClientProvider client={queryClient}>
      <ThemeProvider defaultTheme="system">
        <ToastProvider>
          <SessionProvider bootstrapPath="/api/v1/admin/bootstrap">
            <AuthenticatedContent />
          </SessionProvider>
        </ToastProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
}

// 1. Root route
const rootRoute = createRootRoute({
  component: RootLayout,
  errorComponent: RouteErrorFallback,
  notFoundComponent: () => <NotFoundPage onNavigateHome={() => router.navigate({ to: "/" })} />,
});

// 2. Index route (Dashboard)
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

// 3. Demo route (UI Demo)
const demoRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/demo",
  component: UiDemoPage,
});

// 4. Security route
const securityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/security",
  component: AccountSecurityPage,
});

// 5. Audit viewer route
const auditRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/audit",
  component: AuditViewerPage,
});

// 6. Dummy module route (T03.11 DoD)
const dummyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dummy",
  component: DummyModulePage,
});

// 7. Modules route (T03.12)
const modulesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/modules",
  component: ModuleManagerPage,
});

// 8. Nodes route (Cluster Nodes T02.16)
const nodesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/nodes",
  component: NodesPage,
});

// IP Pools route (T16.05)
const ipPoolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/ip-pools",
  component: AdminIpPoolsPage,
});

// 9. Operations route (Operation Ledger T02.17)
const operationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/operations",
  component: OperationsPage,
});

// 10. Settings route (T03.13)
const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsPage,
});

// 11. Feature Sets route (T04.02)
const featureSetsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/feature-sets",
  component: FeatureSetsPage,
});

// 12. Plans route (T04.04)
const plansRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plans",
  component: PlansPage,
});

// 13. Users route (T04.15)
const usersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users",
  component: UsersPage,
});

// 14. User create wizard route (T04.16)
const userWizardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users/new",
  component: UserCreateWizardPage,
});

// 15. User detail route (T04.17)
const userDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users/$id",
  component: UserDetailPage,
});

// 16. Client dashboard route (T04.18)
const clientRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client",
  component: ClientDashboardPage,
});

// 17. Client domains route (T05.05)
const clientDomainsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/domains",
  component: ClientDomainsPage,
});

// 18. Domains management route (T05.06)
const domainsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/domains",
  component: AdminDomainsPage,
});

// 19. DNS Zone Editor route (T05.11)
const dnsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dns",
  component: DnsEditorPage,
});

// DNS Cluster route (T16.04)
const adminDnsClusterRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/dns-cluster",
  component: AdminDnsClusterPage,
});

// 20. SSL / TLS Certificate route (T05.19)
const sslRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/ssl",
  component: SslPage,
});

// 21. VHost Template Management route (T06.07)
const vhostRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vhost-templates",
  component: VhostTemplatesPage,
});

// 22. PHP Runtime Management route (T06.11)
const phpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/php",
  component: PhpManagementPage,
});

// 23. Web Services Management route (T06.12)
const servicesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/services",
  component: ServicesPage,
});

// 24. Error & Suspended Pages route (T06.13)
const errorPagesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/error-pages",
  component: ErrorPagesPage,
});

// 25. Databases route (T07.10)
const clientDatabasesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/databases",
  component: ClientDatabasesPage,
});

const databasesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/databases",
  component: ClientDatabasesPage,
});

// 26. Admin Database Overview route (T07.11)
const adminDatabasesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/databases",
  component: AdminDatabasesPage,
});

// 27. File Manager route (T08.09)
const clientFilesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/files",
  component: FileManagerPage,
});

const filesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/files",
  component: FileManagerPage,
});

// 28. Disk Usage route (T08.10)
const clientDiskUsageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/disk-usage",
  component: DiskUsagePage,
});

const diskUsageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/disk-usage",
  component: DiskUsagePage,
});

// 29. FTP Accounts route (T08.13)
const clientFtpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/ftp",
  component: FtpAccountsPage,
});

const ftpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/ftp",
  component: FtpAccountsPage,
});

// 30. Backups route (T09.09)
const clientBackupsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/backups",
  component: ClientBackupsPage,
});

const backupsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/backups",
  component: ClientBackupsPage,
});

// 31. Admin Backups route (T09.10)
const adminBackupsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/backups",
  component: AdminBackupsPage,
});

// 32. Cron routes (T10.03)
const clientCronRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/cron",
  component: CronJobsPage,
});

const cronRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/cron",
  component: CronJobsPage,
});

// 33. Log Viewer routes (T10.06)
const clientLogsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/logs",
  component: LogViewerPage,
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: LogViewerPage,
});

const adminLogsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/logs",
  component: LogViewerPage,
});

// 34. Admin System Logs route (T10.07)
const adminSystemLogsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/system-logs",
  component: AdminSystemLogsPage,
});

// 35. Usage routes (T10.10)
const clientUsageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/usage",
  component: ClientUsagePage,
});

const usageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/usage",
  component: ClientUsagePage,
});

const adminUsageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/usage",
  component: AdminUsagePage,
});

// 36. Process routes (T10.14)
const clientProcessesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/client/processes",
  component: ClientProcessesPage,
});

const processesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/processes",
  component: ClientProcessesPage,
});

const adminProcessesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/processes",
  component: AdminProcessesPage,
});

// 37. Server Tools route (T10.15)
const adminServerToolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/server-tools",
  component: AdminServerToolsPage,
});

// 38. WAF routes (T11.04)
const clientWafRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/waf",
  component: ClientWafPage,
});

const adminWafRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/waf",
  component: AdminWafPage,
});

// 44. Migration & Transfer routes (T15.06)
const adminMigrationRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/migration",
  component: MigrationPage,
});

const clientMigrationRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/migration",
  component: MigrationPage,
});

// 45. License routes (T17.13)
const adminLicenseRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/license",
  component: LicensePage,
});

const licenseRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/license",
  component: LicensePage,
});

// 46. License Authority route (T17.06)
const laRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/la",
  component: LaAdminPage,
});

// 47. Notification routes (T18.05)
const adminNotificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/notifications",
  component: NotificationPreferencesPage,
});

const settingsNotificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/notifications",
  component: NotificationPreferencesPage,
});

// 48. Branding routes (T18.06)
const adminBrandingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/branding",
  component: BrandingPage,
});

const brandingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/branding",
  component: BrandingPage,
});

// 49. Update routes (T18.10)
const updateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/update",
  component: UpdatePage,
});

const adminUpdateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/update",
  component: UpdatePage,
});

// 50. API Tokens & Docs routes (T18.11)
const apiTokensRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tokens",
  component: ApiTokensPage,
});

const adminApiTokensRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/api-tokens",
  component: ApiTokensPage,
});
const adminSupportReportRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/support-report",
  component: SupportReportPage,
});

const supportReportRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/support-report",
  component: SupportReportPage,
});

// 51. Firewall routes (T11.08)
const clientFirewallRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/firewall",
  component: ClientFirewallPage,
});

const adminFirewallRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/firewall",
  component: AdminFirewallPage,
});

// 52. Security Center routes (T11.11)
const systemToolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/system-tools",
  component: AdminSystemToolsPage,
});

const adminSecurityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/security",
  component: AdminSecurityCenterPage,
});

const adminSecurityCenterRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/security-center",
  component: AdminSecurityCenterPage,
});

const securityCenterRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/security-center",
  component: AdminSecurityCenterPage,
});

// 53. Apps routes (T12.06 & T12.03)
const appsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps",
  component: AppsPage,
});

const adminAppsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/apps",
  component: AppsPage,
});

// 54. Email & Mail Queue routes (T13.12, T13.10, T13.11)
const emailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/email",
  component: ClientEmailPage,
});

const adminMailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/mail",
  component: AdminMailQueuePage,
});

const adminEmailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/email",
  component: AdminMailQueuePage,
});

// 55. Reseller routes (T14.05, T14.02, T14.07)
const resellerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/reseller",
  component: ResellerPortalPage,
});

const administratorsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/administrators",
  component: AdministratorsPage,
});

const adminResellersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/resellers",
  component: AdminResellersPage,
});

const adminResellerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/reseller",
  component: AdminResellersPage,
});

// 56. License Authority Admin alias
const adminLaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin/la",
  component: LaAdminPage,
});

// 18. Login route
const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: () => {
    const { t } = useTranslation();
    return (
      <LoginPage
        title={t("common.adminTitle")}
        onSuccess={() => router.navigate({ to: "/" })}
      />
    );
  },
});

// 19. Route Tree
const routeTree = rootRoute.addChildren([
  indexRoute,
  demoRoute,
  securityRoute,
  auditRoute,
  dummyRoute,
  modulesRoute,
  nodesRoute,
  ipPoolsRoute,
  operationsRoute,
  settingsRoute,
  featureSetsRoute,
  plansRoute,
  usersRoute,
  userWizardRoute,
  userDetailRoute,
  clientRoute,
  clientDomainsRoute,
  domainsRoute,
  dnsRoute,
  adminDnsClusterRoute,
  sslRoute,
  vhostRoute,
  phpRoute,
  servicesRoute,
  errorPagesRoute,
  clientDatabasesRoute,
  databasesRoute,
  adminDatabasesRoute,
  clientFilesRoute,
  filesRoute,
  clientDiskUsageRoute,
  diskUsageRoute,
  clientFtpRoute,
  ftpRoute,
  clientBackupsRoute,
  backupsRoute,
  adminBackupsRoute,
  clientCronRoute,
  cronRoute,
  clientLogsRoute,
  logsRoute,
  adminLogsRoute,
  adminSystemLogsRoute,
  clientUsageRoute,
  usageRoute,
  adminUsageRoute,
  clientProcessesRoute,
  processesRoute,
  adminProcessesRoute,
  adminServerToolsRoute,
  clientWafRoute,
  adminWafRoute,
  clientFirewallRoute,
  adminFirewallRoute,
  adminSecurityRoute,
  systemToolsRoute,
  adminSecurityCenterRoute,
  securityCenterRoute,
  appsRoute,
  adminAppsRoute,
  emailRoute,
  adminMailRoute,
  adminEmailRoute,
  resellerRoute,
  adminResellersRoute,
  administratorsRoute,
  adminResellerRoute,
  adminMigrationRoute,
  clientMigrationRoute,
  adminLicenseRoute,
  licenseRoute,
  laRoute,
  adminLaRoute,
  adminNotificationsRoute,
  settingsNotificationsRoute,
  adminBrandingRoute,
  brandingRoute,
  updateRoute,
  adminUpdateRoute,
  apiTokensRoute,
  adminApiTokensRoute,
  adminSupportReportRoute,
  supportReportRoute,
  loginRoute,
]);

// Create Router
export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  defaultErrorComponent: RouteErrorFallback,
  notFoundMode: "root",
});
