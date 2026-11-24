import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ClientConfigCard } from "./ClientConfigCard";
import { CreateMailboxDialog } from "./CreateMailboxDialog";
import { MailboxesTable } from "./MailboxesTable";
import { ForwardersTable } from "./ForwardersTable";
import { CreateForwarderDialog } from "./CreateForwarderDialog";
import { AutorespondersTable } from "./AutorespondersTable";
import { CreateAutoresponderDialog } from "./CreateAutoresponderDialog";
import { ClientEmailPage } from "./ClientEmailPage";
import { AdminMailQueuePage } from "./AdminMailQueuePage";
import {
  Mailbox,
  EmailForwarder,
  Autoresponder,
  EmailClientConfig,
  MailQueueItem,
  DeliverabilityCheck,
} from "./types";

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

describe("Email Module UI Components (T13.12, T13.10, T13.11)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockConfig: EmailClientConfig = {
    incoming_server: "mail.example.com",
    incoming_imap_port: 993,
    incoming_pop3_port: 995,
    incoming_security: "SSL/TLS",
    outgoing_server: "mail.example.com",
    outgoing_smtp_port: 465,
    outgoing_submission_port: 587,
    outgoing_security: "SSL/TLS (or STARTTLS)",
    auth_required: true,
  };

  const mockMailboxes: Mailbox[] = [
    {
      id: "mb-1",
      email: "info@example.com",
      username: "info",
      domain: "example.com",
      quota_mb: 2048,
      used_bytes: 524288000,
      created_at: "2026-01-01T00:00:00Z",
      webmail_url: "https://webmail.example.com",
    },
  ];

  const mockForwarders: EmailForwarder[] = [
    {
      id: "fwd-1",
      source: "sales@example.com",
      destination: "external@team.com",
      domain: "example.com",
      created_at: "2026-01-01T00:00:00Z",
    },
  ];

  const mockAutoresponders: Autoresponder[] = [
    {
      id: "ar-1",
      email: "support@example.com",
      domain: "example.com",
      subject: "Away Notice",
      body: "I am out of the office.",
      is_active: true,
    },
  ];

  const mockQueue: MailQueueItem[] = [
    {
      id: "QUEUE123",
      sender: "sender@example.com",
      recipient: "dest@remote.org",
      status: "queued",
      retries: 0,
      queued_at: new Date().toISOString(),
      size_bytes: 4096,
    },
  ];

  const mockDeliverability: DeliverabilityCheck[] = [
    {
      domain: "example.com",
      spf_status: "pass",
      dkim_status: "pass",
      dmarc_status: "pass",
      mx_status: "pass",
      rbl_blacklisted: false,
      rbl_listings: [],
      recommendations: ["DKIM selector 'builtpanel' is active with 2048-bit RSA key."],
    },
  ];

  describe("ClientConfigCard (IMAP/SMTP Settings)", () => {
    it("renders IMAP and SMTP configuration details correctly", () => {
      render(<ClientConfigCard config={mockConfig} domain="example.com" loading={false} />);

      expect(screen.getByTestId("client-config-card")).toBeDefined();
      expect(screen.getByTestId("incoming-server-val").textContent).toBe("mail.example.com");
      expect(screen.getByTestId("imap-port-val").textContent).toBe("993");
      expect(screen.getByTestId("outgoing-server-val").textContent).toBe("mail.example.com");
      expect(screen.getByTestId("smtp-port-val").textContent).toBe("465 (or 587)");
    });

    it("supports copying configuration values to clipboard", () => {
      const writeTextMock = vi.fn().mockResolvedValue(undefined);
      Object.assign(navigator, {
        clipboard: {
          writeText: writeTextMock,
        },
      });

      render(<ClientConfigCard config={mockConfig} domain="example.com" loading={false} />);
      const copyBtn = screen.getByTestId("copy-incoming-server");
      fireEvent.click(copyBtn);

      expect(writeTextMock).toHaveBeenCalledWith("mail.example.com");
    });
  });

  describe("MailboxesTable & CreateMailboxDialog", () => {
    it("renders mailbox list and triggers create modal", () => {
      render(
        <MailboxesTable
          mailboxes={mockMailboxes}
          domain="example.com"
          loading={false}
          onCreateMailbox={vi.fn()}
          onDeleteMailbox={vi.fn()}
          onChangePassword={vi.fn()}
        />
      );

      expect(screen.getByTestId("mailbox-row-mb-1")).toBeDefined();
      expect(screen.getByText("info@example.com")).toBeDefined();
      expect(screen.getByTestId("webmail-btn-mb-1")).toBeDefined();
    });

    it("submits new mailbox data in CreateMailboxDialog", async () => {
      const handleCreate = vi.fn().mockResolvedValue(undefined);
      render(
        <CreateMailboxDialog
          open={true}
          domain="example.com"
          onClose={vi.fn()}
          onCreate={handleCreate}
        />
      );

      fireEvent.change(screen.getByTestId("mailbox-username-input"), {
        target: { value: "contact" },
      });
      fireEvent.click(screen.getByTestId("submit-mailbox-btn"));

      await waitFor(() => {
        expect(handleCreate).toHaveBeenCalledWith(
          expect.objectContaining({
            username: "contact",
            domain: "example.com",
            quota_mb: 1024,
          })
        );
      });
    });
  });

  describe("ForwardersTable & CreateForwarderDialog", () => {
    it("renders forwarder list with destination address", () => {
      render(
        <ForwardersTable
          forwarders={mockForwarders}
          domain="example.com"
          loading={false}
          onCreateForwarder={vi.fn()}
          onDeleteForwarder={vi.fn()}
        />
      );

      expect(screen.getByTestId("forwarder-row-fwd-1")).toBeDefined();
      expect(screen.getByText("sales@example.com")).toBeDefined();
      expect(screen.getByText("external@team.com")).toBeDefined();
    });

    it("creates a new email forwarder via dialog", async () => {
      const handleCreate = vi.fn().mockResolvedValue(undefined);
      render(
        <CreateForwarderDialog
          open={true}
          domain="example.com"
          onClose={vi.fn()}
          onCreate={handleCreate}
        />
      );

      fireEvent.change(screen.getByTestId("forwarder-source-input"), {
        target: { value: "hello" },
      });
      fireEvent.change(screen.getByTestId("forwarder-destination-input"), {
        target: { value: "hello@gmail.com" },
      });
      fireEvent.click(screen.getByTestId("submit-forwarder-btn"));

      await waitFor(() => {
        expect(handleCreate).toHaveBeenCalledWith({
          source: "hello@example.com",
          destination: "hello@gmail.com",
          domain: "example.com",
        });
      });
    });
  });

  describe("AutorespondersTable & CreateAutoresponderDialog", () => {
    it("renders autoresponder entries with active badge", () => {
      render(
        <AutorespondersTable
          autoresponders={mockAutoresponders}
          domain="example.com"
          loading={false}
          onCreateAutoresponder={vi.fn()}
          onDeleteAutoresponder={vi.fn()}
        />
      );

      expect(screen.getByTestId("autoresponder-row-ar-1")).toBeDefined();
      expect(screen.getByText("support@example.com")).toBeDefined();
      expect(screen.getByText("Away Notice")).toBeDefined();
    });

    it("creates autoresponder with subject and body", async () => {
      const handleCreate = vi.fn().mockResolvedValue(undefined);
      render(
        <CreateAutoresponderDialog
          open={true}
          domain="example.com"
          onClose={vi.fn()}
          onCreate={handleCreate}
        />
      );

      fireEvent.change(screen.getByTestId("autoresponder-email-input"), {
        target: { value: "support" },
      });
      fireEvent.change(screen.getByTestId("autoresponder-subject-input"), {
        target: { value: "We are away" },
      });
      fireEvent.click(screen.getByTestId("submit-autoresponder-btn"));

      await waitFor(() => {
        expect(handleCreate).toHaveBeenCalledWith(
          expect.objectContaining({
            email: "support@example.com",
            domain: "example.com",
            subject: "We are away",
          })
        );
      });
    });
  });

  describe("ClientEmailPage Integration", () => {
    it("loads data and switches between tabs", async () => {
      mockApiFetch.mockImplementation((url: string) => {
        if (url.includes("/api/v1/email/mailboxes")) return Promise.resolve(mockMailboxes);
        if (url.includes("/api/v1/email/forwarders")) return Promise.resolve(mockForwarders);
        if (url.includes("/api/v1/email/autoresponders")) return Promise.resolve(mockAutoresponders);
        if (url.includes("/api/v1/email/config")) return Promise.resolve(mockConfig);
        return Promise.resolve([]);
      });

      render(<ClientEmailPage />);

      await waitFor(() => {
        expect(screen.getByTestId("mailboxes-section")).toBeDefined();
        expect(screen.getByText("info@example.com")).toBeDefined();
      });

      // Switch to Forwarders tab
      fireEvent.click(screen.getByTestId("tab-forwarders"));
      await waitFor(() => {
        expect(screen.getByTestId("forwarders-section")).toBeDefined();
        expect(screen.getByText("sales@example.com")).toBeDefined();
      });

      // Switch to Device Config tab
      fireEvent.click(screen.getByTestId("tab-config"));
      await waitFor(() => {
        expect(screen.getByTestId("client-config-card")).toBeDefined();
      });
    });
  });

  describe("AdminMailQueuePage (T13.10 & T13.11)", () => {
    it("renders mail queue and triggers flush action", async () => {
      mockApiFetch.mockImplementation((url: string, opts?: any) => {
        if (url.includes("/api/v1/admin/email/queue/flush")) {
          return Promise.resolve({ success: true, count: 1 });
        }
        if (url.includes("/api/v1/admin/email/queue")) return Promise.resolve(mockQueue);
        if (url.includes("/api/v1/admin/email/deliverability")) return Promise.resolve(mockDeliverability);
        return Promise.resolve([]);
      });

      render(<AdminMailQueuePage />);

      await waitFor(() => {
        expect(screen.getByTestId("queue-row-QUEUE123")).toBeDefined();
        expect(screen.getByText("QUEUE123")).toBeDefined();
        expect(screen.getByText("sender@example.com")).toBeDefined();
      });

      // Flush queue
      fireEvent.click(screen.getByTestId("flush-queue-btn"));
      await waitFor(() => {
        expect(mockApiFetch).toHaveBeenCalledWith(
          "/api/v1/admin/email/queue/flush",
          expect.objectContaining({ method: "POST" })
        );
      });

      // Switch to Deliverability tab
      fireEvent.click(screen.getByTestId("tab-admin-deliverability"));
      await waitFor(() => {
        expect(screen.getByTestId("deliverability-card-example.com")).toBeDefined();
        expect(screen.getByText(/DKIM selector/i)).toBeDefined();
      });
    });
  });
});
