import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { CommandPalette } from "./CommandPalette";
import { CommandPaletteTrigger } from "./CommandPaletteTrigger";

const mockRemoteResults = [
  {
    id: "domain-apexstore",
    title: "apexstore.com",
    subtitle: "Primary Domain (apex_store)",
    category: "domains",
    href: "/domains",
    badge: "Domain",
  },
  {
    id: "user-apex",
    title: "apex_store",
    subtitle: "Tenant Account",
    category: "users",
    href: "/users",
    badge: "Tenant",
  },
];

vi.mock("@builtpanel/api", () => ({
  apiFetch: vi.fn(async (url: string) => {
    if (url.includes("/api/v1/search?q=apex")) {
      return mockRemoteResults;
    }
    return [];
  }),
}));

describe("Global Search & Command Palette (T18.09)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("CommandPaletteTrigger", () => {
    it("renders trigger button and handles click", () => {
      const handleClick = vi.fn();
      render(<CommandPaletteTrigger onClick={handleClick} />);

      const triggerBtn = screen.getByTestId("command-palette-trigger");
      expect(triggerBtn).toBeDefined();

      fireEvent.click(triggerBtn);
      expect(handleClick).toHaveBeenCalledTimes(1);
    });
  });

  describe("CommandPalette", () => {
    it("opens on Ctrl+K keyboard shortcut and closes on Escape", async () => {
      render(<CommandPalette />);

      // Initially closed
      expect(screen.queryByTestId("command-palette-dialog")).toBeNull();

      // Trigger Ctrl+K
      fireEvent.keyDown(window, { key: "k", ctrlKey: true });

      await waitFor(() => {
        expect(screen.getByTestId("command-palette-dialog")).toBeDefined();
        expect(screen.getByTestId("command-palette-input")).toBeDefined();
      });

      // Press Escape
      fireEvent.keyDown(screen.getByTestId("command-palette-input"), { key: "Escape" });

      await waitFor(() => {
        expect(screen.queryByTestId("command-palette-dialog")).toBeNull();
      });
    });

    it("filters static pages by keyword", async () => {
      render(<CommandPalette open={true} onOpenChange={vi.fn()} />);

      const input = screen.getByTestId("command-palette-input");
      fireEvent.change(input, { target: { value: "domains" } });

      await waitFor(() => {
        expect(screen.getByTestId("search-result-item-page-domains")).toBeDefined();
      });
    });

    it("fetches remote search results and selects item on Enter", async () => {
      const handleNavigate = vi.fn();
      render(
        <CommandPalette
          open={true}
          onOpenChange={vi.fn()}
          onNavigate={handleNavigate}
        />
      );

      const input = screen.getByTestId("command-palette-input");
      fireEvent.change(input, { target: { value: "apex" } });

      await waitFor(() => {
        expect(screen.getByText("apexstore.com")).toBeDefined();
      });

      // Press Enter to select first item
      fireEvent.keyDown(input, { key: "Enter" });

      expect(handleNavigate).toHaveBeenCalledWith("/domains");
    });

    it("displays empty state when no results match query", async () => {
      render(<CommandPalette open={true} onOpenChange={vi.fn()} />);

      const input = screen.getByTestId("command-palette-input");
      fireEvent.change(input, { target: { value: "nonexistenttermxyz" } });

      await waitFor(() => {
        expect(screen.getByTestId("command-palette-empty")).toBeDefined();
      });
    });
  });
});
