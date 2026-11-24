import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AdminQuickstartChecklist } from "./AdminQuickstartChecklist";
import { ClientWelcomeTour } from "./ClientWelcomeTour";

// Mock @builtpanel/api
vi.mock("@builtpanel/api", () => ({
  apiFetch: vi.fn(async (url: string) => {
    if (url === "/api/v1/onboarding/admin") {
      return { dismissed: false, completed_steps: ["add_node"] };
    }
    if (url === "/api/v1/onboarding/client") {
      return { dismissed: false };
    }
    return { ok: true };
  }),
}));

describe("Onboarding & Tour (T18.07)", () => {
  let mockStorage: Record<string, string> = {};

  beforeEach(() => {
    mockStorage = {};
    const storageMock = {
      getItem: vi.fn((key: string) => mockStorage[key] ?? null),
      setItem: vi.fn((key: string, value: string) => {
        mockStorage[key] = String(value);
      }),
      removeItem: vi.fn((key: string) => {
        delete mockStorage[key];
      }),
      clear: vi.fn(() => {
        mockStorage = {};
      }),
    };

    Object.defineProperty(window, "localStorage", {
      value: storageMock,
      writable: true,
      configurable: true,
    });
    Object.defineProperty(globalThis, "localStorage", {
      value: storageMock,
      writable: true,
      configurable: true,
    });
  });

  describe("AdminQuickstartChecklist", () => {
    it("renders the 4 setup tasks with progress", async () => {
      render(<AdminQuickstartChecklist />);

      expect(screen.getByTestId("admin-quickstart-card")).toBeInTheDocument();
      expect(screen.getByTestId("quickstart-title")).toBeInTheDocument();
      expect(screen.getByTestId("quickstart-step-add_node")).toBeInTheDocument();
      expect(screen.getByTestId("quickstart-step-create_plan")).toBeInTheDocument();
      expect(screen.getByTestId("quickstart-step-configure_dns")).toBeInTheDocument();
      expect(screen.getByTestId("quickstart-step-setup_smtp")).toBeInTheDocument();
    });

    it("toggles step completed state on click", async () => {
      render(<AdminQuickstartChecklist />);

      const toggleBtn = screen.getByTestId("btn-toggle-step-create_plan");
      fireEvent.click(toggleBtn);

      await waitFor(() => {
        expect(localStorage.setItem).toHaveBeenCalledWith(
          "builtpanel-admin-onboarding-completed-steps",
          expect.stringContaining('"create_plan":true')
        );
      });
    });

    it("hides and persists dismissal when dismissed (DoD)", async () => {
      const onDismiss = vi.fn();
      render(<AdminQuickstartChecklist onDismissCallback={onDismiss} />);

      const dismissBtn = screen.getByTestId("btn-dismiss-quickstart");
      fireEvent.click(dismissBtn);

      await waitFor(() => {
        expect(screen.queryByTestId("admin-quickstart-card")).not.toBeInTheDocument();
        expect(localStorage.setItem).toHaveBeenCalledWith(
          "builtpanel-admin-onboarding-dismissed",
          "true"
        );
        expect(onDismiss).toHaveBeenCalled();
      });
    });

    it("does not render when storage indicates already dismissed", () => {
      mockStorage["builtpanel-admin-onboarding-dismissed"] = "true";
      render(<AdminQuickstartChecklist />);
      expect(screen.queryByTestId("admin-quickstart-card")).not.toBeInTheDocument();
    });
  });

  describe("ClientWelcomeTour", () => {
    it("renders tour step 1 and navigates to step 2", async () => {
      render(<ClientWelcomeTour />);

      expect(screen.getByTestId("client-welcome-tour-card")).toBeInTheDocument();
      expect(screen.getByTestId("client-tour-title")).toBeInTheDocument();

      // Step 1 active
      expect(screen.getByTestId("btn-tour-goto-tour_domains")).toBeInTheDocument();

      // Click Next
      const nextBtn = screen.getByTestId("btn-tour-next");
      fireEvent.click(nextBtn);

      // Now Step 2 active
      expect(screen.getByTestId("btn-tour-goto-tour_files")).toBeInTheDocument();

      // Previous button enabled
      const prevBtn = screen.getByTestId("btn-tour-prev");
      expect(prevBtn).not.toBeDisabled();
      fireEvent.click(prevBtn);

      // Back to step 1
      expect(screen.getByTestId("btn-tour-goto-tour_domains")).toBeInTheDocument();
    });

    it("hides and persists dismissal when dismissed (DoD)", async () => {
      const onDismiss = vi.fn();
      render(<ClientWelcomeTour onDismissCallback={onDismiss} />);

      const dismissBtn = screen.getByTestId("btn-dismiss-client-tour");
      fireEvent.click(dismissBtn);

      await waitFor(() => {
        expect(screen.queryByTestId("client-welcome-tour-card")).not.toBeInTheDocument();
        expect(localStorage.setItem).toHaveBeenCalledWith(
          "builtpanel-client-tour-dismissed",
          "true"
        );
        expect(onDismiss).toHaveBeenCalled();
      });
    });

    it("does not render when storage indicates already dismissed", () => {
      mockStorage["builtpanel-client-tour-dismissed"] = "true";
      render(<ClientWelcomeTour />);
      expect(screen.queryByTestId("client-welcome-tour-card")).not.toBeInTheDocument();
    });
  });
});
