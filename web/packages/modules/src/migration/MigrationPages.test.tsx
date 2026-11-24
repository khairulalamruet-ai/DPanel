import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { TransferStepTimeline } from "./TransferStepTimeline";
import { TransferLogDialog } from "./TransferLogDialog";
import { NewTransferWizardDialog } from "./NewTransferWizardDialog";
import { TransfersTable } from "./TransfersTable";
import { MigrationPage } from "./MigrationPage";
import { TransferJob, TransferStep } from "./types";

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

describe("Migration & Transfer UI (T15.06 DoD)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockSteps: TransferStep[] = [
    {
      id: "step-1",
      name: "Connecting to Remote Server via SSH",
      status: "completed",
      started_at: "2026-01-01T00:00:00Z",
      completed_at: "2026-01-01T00:01:00Z",
      details: "Authenticated successfully as root via port 22",
    },
    {
      id: "step-2",
      name: "Restoring MySQL Databases",
      status: "running",
      started_at: "2026-01-01T00:01:00Z",
      details: "Importing database structure",
    },
    {
      id: "step-3",
      name: "Configuring DNS & SSL Certificates",
      status: "pending",
    },
  ];

  const mockJob: TransferJob = {
    id: "TRF-TEST101",
    source_type: "cpanel",
    source_host: "cpanel.source.com",
    tenant_username: "testuser",
    target_domain: "testdomain.com",
    status: "running",
    progress_percent: 45,
    current_step: "Restoring MySQL Databases",
    size_bytes: 524288000,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:02:00Z",
    steps: mockSteps,
    logs: [
      "Connecting to remote host cpanel.source.com on port 22...",
      "SSH handshake verified.",
      "RESTORED user testuser",
    ],
  };

  describe("TransferStepTimeline", () => {
    it("renders step names and statuses correctly", () => {
      render(<TransferStepTimeline steps={mockSteps} />);

      expect(screen.getByTestId("transfer-step-timeline")).toBeDefined();
      expect(screen.getByText("Connecting to Remote Server via SSH")).toBeDefined();
      expect(screen.getByText("Restoring MySQL Databases")).toBeDefined();
      expect(screen.getByText("Configuring DNS & SSL Certificates")).toBeDefined();
    });
  });

  describe("TransferLogDialog (OpenPanel /import/transfers style live log & steps)", () => {
    it("renders transfer overview, step timeline, and console logs", () => {
      render(
        <TransferLogDialog
          job={mockJob}
          open={true}
          onClose={vi.fn()}
          onRetry={vi.fn()}
          onCancel={vi.fn()}
        />
      );

      expect(screen.getByTestId("transfer-log-dialog")).toBeDefined();
      expect(screen.getByTestId("transfer-status-badge").textContent).toBe("RUNNING");
      expect(screen.getByTestId("transfer-log-console")).toBeDefined();
      expect(screen.getByText(/SSH handshake verified/i)).toBeDefined();
    });

    it("copies logs to clipboard", () => {
      const writeTextMock = vi.fn().mockResolvedValue(undefined);
      Object.assign(navigator, {
        clipboard: {
          writeText: writeTextMock,
        },
      });

      render(
        <TransferLogDialog
          job={mockJob}
          open={true}
          onClose={vi.fn()}
        />
      );

      fireEvent.click(screen.getByTestId("btn-copy-transfer-logs"));
      expect(writeTextMock).toHaveBeenCalledWith(mockJob.logs.join("\n"));
    });
  });

  describe("NewTransferWizardDialog", () => {
    it("submits remote migration transfer form", async () => {
      const handleSubmit = vi.fn().mockResolvedValue(undefined);
      render(
        <NewTransferWizardDialog
          open={true}
          onClose={vi.fn()}
          onSubmit={handleSubmit}
        />
      );

      // Select source
      fireEvent.click(screen.getByTestId("source-type-cpanel"));

      // Fill connection details
      fireEvent.change(screen.getByTestId("input-migration-host"), {
        target: { value: "192.168.1.50" },
      });
      fireEvent.change(screen.getByTestId("input-migration-password"), {
        target: { value: "secret123" },
      });
      fireEvent.change(screen.getByTestId("input-migration-target-domain"), {
        target: { value: "clientportal.org" },
      });
      fireEvent.change(screen.getByTestId("input-migration-tenant-user"), {
        target: { value: "clientportal" },
      });

      fireEvent.click(screen.getByTestId("btn-start-transfer"));

      await waitFor(() => {
        expect(handleSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            source_type: "cpanel",
            remote_host: "192.168.1.50",
            target_domain: "clientportal.org",
            tenant_username: "clientportal",
          })
        );
      });
    });
  });

  describe("TransfersTable", () => {
    it("renders job rows and filters by status", () => {
      const completedJob: TransferJob = {
        ...mockJob,
        id: "TRF-COMPLETED",
        status: "completed",
        progress_percent: 100,
      };

      const handleOpenLogs = vi.fn();
      render(
        <TransfersTable
          jobs={[mockJob, completedJob]}
          loading={false}
          onOpenLogs={handleOpenLogs}
          onRetry={vi.fn()}
          onCancel={vi.fn()}
          onDelete={vi.fn()}
        />
      );

      expect(screen.getByTestId("transfer-row-TRF-TEST101")).toBeDefined();
      expect(screen.getByTestId("transfer-row-TRF-COMPLETED")).toBeDefined();

      // Filter by completed
      fireEvent.click(screen.getByTestId("filter-completed"));
      expect(screen.queryByTestId("transfer-row-TRF-TEST101")).toBeNull();
      expect(screen.getByTestId("transfer-row-TRF-COMPLETED")).toBeDefined();
    });
  });

  describe("MigrationPage Integration", () => {
    it("fetches transfers, renders metrics, and opens dialogs", async () => {
      mockApiFetch.mockResolvedValue([mockJob]);

      render(<MigrationPage />);

      await waitFor(() => {
        expect(screen.getByTestId("migration-page-title")).toBeDefined();
        expect(screen.getByTestId("stat-active-transfers").textContent).toBe("1");
      });

      // Open new transfer wizard
      fireEvent.click(screen.getByTestId("btn-open-new-transfer"));
      await waitFor(() => {
        expect(screen.getByTestId("new-transfer-dialog")).toBeDefined();
      });
    });
  });
});
