import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { CatalogGrid } from "./CatalogGrid";
import { InstallWizardDialog } from "./InstallWizardDialog";
import { InstalledAppsTable } from "./InstalledAppsTable";
import { WordPressManagerDialog } from "./WordPressManagerDialog";
import { AppsPage } from "./AppsPage";
import { CatalogApp, InstalledApp, WordPressDetails } from "./types";

// Mock @builtpanel/api
const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));

// Mock @builtpanel/i18n
vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string, defaultVal?: string | Record<string, any>) => {
      if (typeof defaultVal === "string") return defaultVal;
      return key;
    },
    i18n: { language: "en" },
  }),
}));

// Mock @builtpanel/ui toast
const mockToast = vi.fn();
vi.mock("@builtpanel/ui", async () => {
  const actual = await vi.importActual<any>("@builtpanel/ui");
  return {
    ...actual,
    useToast: () => ({ toast: mockToast }),
  };
});

describe("Autoinstaller & Apps Management UI (T12.06 & T12.03)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockCatalog: CatalogApp[] = [
    {
      id: "wordpress",
      name: "WordPress",
      category: "cms",
      version: "6.7.1",
      description: "World's most popular blogging and CMS engine.",
      icon: "wordpress",
      requires_db: true,
      requires_php: true,
      min_php_version: "8.1",
      documentation_url: "https://wordpress.org",
    },
    {
      id: "laravel",
      name: "Laravel Skeleton",
      category: "framework",
      version: "11.31.0",
      description: "Modern PHP framework for web artisans.",
      icon: "laravel",
      requires_db: true,
      requires_php: true,
      min_php_version: "8.2",
    },
    {
      id: "static",
      name: "Static HTML5 Starter",
      category: "static",
      version: "1.0.0",
      description: "Fast static website starter.",
      icon: "static",
      requires_db: false,
      requires_php: false,
    },
  ];

  const mockInstalled: InstalledApp[] = [
    {
      id: "app-wp-01",
      catalog_app_id: "wordpress",
      name: "WordPress",
      domain: "client-site.com",
      path: "/",
      url: "https://client-site.com",
      admin_url: "https://client-site.com/wp-admin",
      version: "6.7.0",
      latest_version: "6.7.1",
      has_update: true,
      status: "running",
      db_name: "wp_clientsite",
      db_user: "wp_user",
      installed_at: "2026-11-10 12:00:00",
      auto_update: true,
    },
  ];

  const mockWpDetails: WordPressDetails = {
    app_id: "app-wp-01",
    core_version: "6.7.0",
    latest_core_version: "6.7.1",
    maintenance_mode: false,
    xmlrpc_disabled: true,
    debug_mode: false,
    plugins: [
      {
        slug: "woocommerce",
        name: "WooCommerce",
        version: "8.5.1",
        latest_version: "8.6.0",
        has_update: true,
        active: true,
        author: "Automattic",
      },
      {
        slug: "wordpress-seo",
        name: "Yoast SEO",
        version: "22.0",
        latest_version: "22.0",
        has_update: false,
        active: true,
        author: "Team Yoast",
      },
    ],
    themes: [
      {
        slug: "twentytwentyfour",
        name: "Twenty Twenty-Four",
        version: "1.0",
        active: true,
      },
    ],
  };

  it("CatalogGrid: renders catalog apps and handles search and selection", () => {
    const handleSelect = vi.fn();
    render(<CatalogGrid catalog={mockCatalog} onSelectApp={handleSelect} />);

    expect(screen.getByTestId("catalog-grid")).toBeDefined();
    expect(screen.getByTestId("catalog-card-wordpress")).toBeDefined();
    expect(screen.getByTestId("catalog-card-laravel")).toBeDefined();
    expect(screen.getByTestId("catalog-card-static")).toBeDefined();

    // Select WordPress to install
    const installBtn = screen.getByTestId("btn-install-wordpress");
    fireEvent.click(installBtn);
    expect(handleSelect).toHaveBeenCalledWith(
      expect.objectContaining({ id: "wordpress" })
    );

    // Search filter
    const searchInput = screen.getByTestId("input-search-catalog");
    fireEvent.change(searchInput, { target: { value: "Laravel" } });
    expect(screen.queryByTestId("catalog-card-wordpress")).toBeNull();
    expect(screen.getByTestId("catalog-card-laravel")).toBeDefined();
  });

  it("InstallWizardDialog: configures installation parameters and submits", async () => {
    const handleInstall = vi.fn().mockResolvedValue(mockInstalled[0]);
    const handleSuccess = vi.fn();
    const handleClose = vi.fn();

    render(
      <InstallWizardDialog
        open={true}
        app={mockCatalog[0]}
        domains={["client-site.com"]}
        onClose={handleClose}
        onInstall={handleInstall}
        onSuccess={handleSuccess}
      />
    );

    expect(screen.getByTestId("install-wizard-title")).toBeDefined();
    expect(screen.getByTestId("select-app-domain")).toBeDefined();
    expect(screen.getByTestId("input-site-name")).toBeDefined();

    // There is no preset login: the e-mail and the password are given
    fireEvent.change(screen.getByTestId("input-admin-email"), { target: { value: "me@client-site.com" } });
    fireEvent.change(screen.getByTestId("input-admin-password"), { target: { value: "A-long-unique-pass-17" } });

    // Submit form
    const submitBtn = screen.getByTestId("btn-submit-install");
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(handleInstall).toHaveBeenCalledWith(
        expect.objectContaining({
          catalog_app_id: "wordpress",
          domain: "client-site.com",
        })
      );
    });

    // Check success screen
    await waitFor(() => {
      expect(screen.getByTestId("install-success-view")).toBeDefined();
      expect(screen.getByTestId("link-installed-site")).toBeDefined();
    });
  });

  it("InstalledAppsTable: renders list of tenant applications (DoD)", () => {
    const handleOpenManage = vi.fn();
    const handleUninstall = vi.fn();

    render(
      <InstalledAppsTable
        apps={mockInstalled}
        onOpenManage={handleOpenManage}
        onUninstall={handleUninstall}
      />
    );

    expect(screen.getByTestId("installed-apps-table")).toBeDefined();
    expect(screen.getByTestId("installed-app-name-app-wp-01")).toHaveTextContent("WordPress");
    expect(screen.getByTestId("link-visit-app-app-wp-01")).toBeDefined();

    // Manage button
    const manageBtn = screen.getByTestId("btn-manage-app-app-wp-01");
    fireEvent.click(manageBtn);
    expect(handleOpenManage).toHaveBeenCalledWith(mockInstalled[0]);
  });

  it("WordPressManagerDialog: meets T12.03 DoD by allowing plugin updates from UI", async () => {
    const handleUpdatePlugin = vi.fn().mockResolvedValue(undefined);
    const handleToggleMaint = vi.fn();
    const handleClose = vi.fn();

    render(
      <WordPressManagerDialog
        open={true}
        app={mockInstalled[0]}
        details={mockWpDetails}
        onClose={handleClose}
        onUpdatePlugin={handleUpdatePlugin}
        onToggleMaintenance={handleToggleMaint}
        onScan={vi.fn()}
        onHarden={vi.fn()}
      />
    );

    expect(screen.getByTestId("wp-manager-title")).toBeDefined();

    // Switch to Plugins tab
    const pluginsTab = screen.getByTestId("tab-wp-plugins");
    fireEvent.click(pluginsTab);

    expect(screen.getByTestId("wp-plugins-table")).toBeDefined();
    expect(screen.getByTestId("plugin-row-woocommerce")).toBeDefined();
    expect(screen.getByTestId("badge-update-woocommerce")).toBeDefined();

    // Click Update button for WooCommerce (T12.03 DoD)
    const updateBtn = screen.getByTestId("btn-update-plugin-woocommerce");
    fireEvent.click(updateBtn);

    await waitFor(() => {
      expect(handleUpdatePlugin).toHaveBeenCalledWith("app-wp-01", "woocommerce");
    });
  });

  it("AppsPage: fetches catalog and installed apps and enables tab switching", async () => {
    mockApiFetch.mockImplementation(async (url: string) => {
      if (url.includes("/apps/catalog")) {
        return {
          items: mockCatalog.map((c) => ({
            slug: c.id,
            name: c.name,
            category: c.category,
            version: c.version,
            description: c.description,
            document_root: "",
            needs_database: c.requires_db,
            needs_admin: true,
          })),
        };
      }
      if (url.endsWith("/api/v1/client/apps")) {
        return {
          items: mockInstalled.map((a) => ({
            id: a.id,
            kind: a.catalog_app_id,
            name: a.name,
            path: a.path,
            fqdn: a.domain,
            domain_id: "dom_01",
            version: a.version,
            status: "active",
            created_at: 1_790_000_000,
          })),
        };
      }
      if (url.endsWith("/api/v1/client/domains")) return { items: [{ id: "dom_01", fqdn: "client-site.com", unicode: "client-site.com" }] };
      return {};
    });

    render(<AppsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("apps-page-title")).toBeDefined();
    });

    // Installed tab is default
    expect(screen.getByTestId("installed-apps-table")).toBeDefined();

    // Switch to catalog tab
    const catalogTab = screen.getByTestId("tab-app-catalog");
    fireEvent.click(catalogTab);

    expect(screen.getByTestId("catalog-grid")).toBeDefined();
    expect(screen.getByTestId("catalog-card-wordpress")).toBeDefined();
  });
});
