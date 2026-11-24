import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { AppInstallerPage } from "./AppInstallerPage";
import { AdminSoftware } from "./types";

const base = { license: "MIT", homepage: "", provided_by: "", reason: "", memory_mb: 128, version: "1.0.0" };
let apps: AdminSoftware[];
const calls: { url: string; method?: string }[] = [];

const mockApiFetch = vi.fn(async (url: string, opts?: any) => {
  calls.push({ url, method: opts?.method });
  if (url === "/api/v1/admin/marketplace") return apps;
  const slug = decodeURIComponent(url.split("/").pop() ?? "");
  apps = apps.map((a) => (a.slug === slug ? { ...a, offered: opts?.method === "POST" } : a));
  return {};
});

vi.mock("@builtpanel/api", () => ({
  apiFetch: (url: string, opts?: any) => mockApiFetch(url, opts),
}));

const renderPage = () =>
  render(
    <ToastProvider>
      <AppInstallerPage />
    </ToastProvider>,
  );

describe("Install an App (administrator)", () => {
  beforeEach(() => {
    calls.length = 0;
    apps = [
      { ...base, slug: "redis", name: "Redis", category: "databases", category_name: "Databases", description: "A cache.", kind: "container", offered: false },
      { ...base, slug: "nginx", name: "Nginx", category: "web_servers", category_name: "Web servers", description: "A web server.", kind: "container", offered: true },
      { ...base, slug: "php", name: "PHP", category: "server_side_scripting", category_name: "Server side scripting", description: "PHP.", kind: "builtin", provided_by: "PHP settings", offered: false },
    ];
  });

  it("lists the software with Install, Remove and a note for what the panel has", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("app-redis")).toBeInTheDocument());
    expect(screen.getByTestId("install-redis")).toBeInTheDocument();
    expect(screen.getByTestId("remove-nginx")).toBeInTheDocument();
    expect(screen.getByTestId("offered-nginx")).toBeInTheDocument();
    expect(screen.queryByTestId("install-php")).toBeNull();
    expect(screen.getByTestId("app-php")).toHaveTextContent("PHP settings");
  });

  it("offers a software to customers and takes it back", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("install-redis")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("install-redis"));
    await waitFor(() => expect(screen.getByTestId("remove-redis")).toBeInTheDocument());
    expect(calls.some((c) => c.url === "/api/v1/admin/marketplace/redis" && c.method === "POST")).toBe(true);
    fireEvent.click(screen.getByTestId("remove-redis"));
    await waitFor(() => expect(screen.getByTestId("install-redis")).toBeInTheDocument());
    expect(calls.some((c) => c.url === "/api/v1/admin/marketplace/redis" && c.method === "DELETE")).toBe(true);
  });

  it("filters by category and by search text", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("app-redis")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("category-web_servers"));
    expect(screen.queryByTestId("app-redis")).toBeNull();
    expect(screen.getByTestId("app-nginx")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("category-all"));
    fireEvent.change(screen.getByTestId("app-search"), { target: { value: "cache" } });
    expect(screen.getByTestId("app-redis")).toBeInTheDocument();
    expect(screen.queryByTestId("app-nginx")).toBeNull();
  });
});
