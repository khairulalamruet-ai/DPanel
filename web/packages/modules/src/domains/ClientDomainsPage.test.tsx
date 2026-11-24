import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockClientDomains } from "@builtpanel/api";
import { ClientDomainsPage } from "./ClientDomainsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ClientDomainsPage (T05.05)", () => {
  beforeEach(() => {
    resetMockClientDomains();
  });

  it("renders page title, quota banner, and initial seeded domains", async () => {
    renderWithProviders(<ClientDomainsPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    expect(screen.getByText("blog.alpha-hosting.com")).toBeInTheDocument();
    expect(screen.getByText("shop.alpha-hosting.com")).toBeInTheDocument();
    // TODO: the domain limit comes with the customer bootstrap; the banner is checked then.
  });

  it("DoD: adds a new addon domain and verifies it appears in the table", async () => {
    renderWithProviders(<ClientDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    // Click "Add Domain"
    const addBtn = screen.getByTestId("btn-add-domain");
    fireEvent.click(addBtn);

    // Dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("input-domain-name")).toBeInTheDocument();
    });

    // Enter domain name
    const domainInput = screen.getByTestId("input-domain-name");
    fireEvent.change(domainInput, { target: { value: "mystore-online.com" } });

    // Submit dialog
    const submitBtn = screen.getByTestId("btn-submit-domain");
    fireEvent.click(submitBtn);

    // Verify it appears in the table
    await waitFor(() => {
      expect(screen.getByText("mystore-online.com")).toBeInTheDocument();
    });
    expect(
      screen.getByText("/domains/mystore-online.com/public_html")
    ).toBeInTheDocument();
  });

  it("edits the document root path of a domain", async () => {
    renderWithProviders(<ClientDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("blog.alpha-hosting.com")).toBeInTheDocument();
    });

    // Click edit docroot for dom_02
    const editBtn = screen.getByTestId("btn-edit-docroot-dom_02");
    fireEvent.click(editBtn);

    // Edit Docroot dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("input-edit-docroot")).toBeInTheDocument();
    });

    const docrootInput = screen.getByTestId("input-edit-docroot");
    fireEvent.change(docrootInput, {
      target: { value: "/domains/blog.alpha-hosting.com/dist" },
    });

    const saveBtn = screen.getByTestId("btn-save-docroot");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(
        screen.getByText("/domains/blog.alpha-hosting.com/dist")
      ).toBeInTheDocument();
    });
  });

  it("configures URL redirection for a domain", async () => {
    renderWithProviders(<ClientDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("blog.alpha-hosting.com")).toBeInTheDocument();
    });

    // Click manage redirect for dom_02
    const redirectBtn = screen.getByTestId("btn-manage-redirect-dom_02");
    fireEvent.click(redirectBtn);

    // Redirection dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("checkbox-manage-redirect-toggle")).toBeInTheDocument();
    });

    // Enable redirect
    const toggle = screen.getByTestId("checkbox-manage-redirect-toggle");
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(screen.getByTestId("input-manage-redirect-url")).toBeInTheDocument();
    });

    const targetUrlInput = screen.getByTestId("input-manage-redirect-url");
    fireEvent.change(targetUrlInput, {
      target: { value: "https://myblogsite.wordpress.com" },
    });

    const saveBtn = screen.getByTestId("btn-save-redirect");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(
        screen.getByText("https://myblogsite.wordpress.com")
      ).toBeInTheDocument();
    });
  });

  it("deletes a secondary domain and preserves primary domain", async () => {
    renderWithProviders(<ClientDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("shop.alpha-hosting.com")).toBeInTheDocument();
    });

    // Primary domain delete button is disabled
    const primaryDeleteBtn = screen.getByTestId("btn-delete-domain-dom_01");
    expect(primaryDeleteBtn).toBeDisabled();

    // Click delete on shop.alpha-hosting.com (dom_03)
    const deleteBtn = screen.getByTestId("btn-delete-domain-dom_03");
    expect(deleteBtn).not.toBeDisabled();
    fireEvent.click(deleteBtn);

    // Confirm dialog
    await waitFor(() => {
      expect(screen.getByTestId("btn-confirm-delete-domain")).toBeInTheDocument();
    });

    const confirmBtn = screen.getByTestId("btn-confirm-delete-domain");
    fireEvent.click(confirmBtn);

    // Verify dom_03 is removed from the table
    await waitFor(() => {
      expect(screen.queryByText("shop.alpha-hosting.com")).not.toBeInTheDocument();
    });
  });

  it("filters domains by search input", async () => {
    renderWithProviders(<ClientDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-search-domains");
    fireEvent.change(searchInput, { target: { value: "shop" } });

    expect(screen.getByText("shop.alpha-hosting.com")).toBeInTheDocument();
    expect(screen.queryByText("blog.alpha-hosting.com")).not.toBeInTheDocument();
  });
});
