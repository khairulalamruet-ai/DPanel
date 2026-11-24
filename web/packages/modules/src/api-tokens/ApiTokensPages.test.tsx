import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { ApiTokensPage } from "./ApiTokensPage";
import { CreateTokenDialog } from "./CreateTokenDialog";
import { ApiDocsExplorer } from "./ApiDocsExplorer";
import { ApiTokenItem } from "./types";

const mockTokens: ApiTokenItem[] = [
  {
    id: "tok-test-01",
    name: "WHMCS Billing Integration",
    token_preview: "dhp_live_9f82...104a",
    scopes: ["users:read", "billing:whmcs"],
    created_at: new Date().toISOString(),
    expires_at: null,
    last_used_at: null,
  },
];

const mockApiFetch = vi.fn(async (url: string, opts?: any) => {
  if (url === "/api/v1/tokens" && (!opts || opts.method === "GET")) {
    return { tokens: mockTokens };
  }
  if (url === "/api/v1/tokens" && opts?.method === "POST") {
    const body = JSON.parse(opts.body);
    return {
      token: {
        id: "tok-new-02",
        name: body.name,
        token_preview: "dhp_live_1234...5678",
        scopes: body.scopes,
        created_at: new Date().toISOString(),
        expires_at: null,
        last_used_at: null,
      },
      secret_key: "dhp_live_plaintext_super_secret_key_12345",
    };
  }
  if (url.startsWith("/api/v1/tokens/") && opts?.method === "DELETE") {
    return { ok: true };
  }
  if (url === "/api/v1/tokens/test-call") {
    const body = JSON.parse(opts.body);
    if (body.endpoint === "/users") {
      return {
        status: 200,
        message: "Token authorized for scope 'users:read'",
        data: { users_count: 3, users: ["admin", "client_alpha"] },
      };
    }
    if (body.endpoint === "/admin/nodes") {
      return {
        status: 403,
        message: "Forbidden: Token lacks required scope 'cluster:admin'",
      };
    }
  }
  return { ok: true };
});

vi.mock("@builtpanel/api", () => ({
  apiFetch: (url: string, opts?: any) => mockApiFetch(url, opts),
}));

describe("API Tokens & Public API Docs (T18.11)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders API tokens page with token table and OpenAPI explorer", async () => {
    render(
      <ToastProvider>
        <ApiTokensPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByTestId("api-tokens-page")).toBeInTheDocument();
      expect(screen.getByText("WHMCS Billing Integration")).toBeInTheDocument();
      expect(screen.getByText("dhp_live_9f82...104a")).toBeInTheDocument();
      expect(screen.getByTestId("api-docs-explorer-card")).toBeInTheDocument();
    });
  });

  it("generates a new token and displays the secret key once", async () => {
    const onCreated = vi.fn();
    const onClose = vi.fn();

    render(
      <ToastProvider>
        <CreateTokenDialog open={true} onClose={onClose} onCreated={onCreated} />
      </ToastProvider>
    );

    // Enter name
    const nameInput = screen.getByTestId("input-token-name");
    fireEvent.change(nameInput, { target: { value: "Backup Automation CLI" } });

    // Submit
    const submitBtn = screen.getByTestId("btn-submit-create-token");
    fireEvent.click(submitBtn);

    // Revealed view
    await waitFor(() => {
      expect(screen.getByTestId("token-secret-revealed-view")).toBeInTheDocument();
      expect(screen.getByTestId("created-secret-input")).toHaveValue(
        "dhp_live_plaintext_super_secret_key_12345"
      );
      expect(onCreated).toHaveBeenCalled();
    });
  });

  it("executes GET /users with token and receives 200 OK (DoD)", async () => {
    render(
      <ToastProvider>
        <ApiDocsExplorer tokens={mockTokens} selectedTokenId="tok-test-01" />
      </ToastProvider>
    );

    // Select GET /users
    const usersEndpointBtn = screen.getByTestId("endpoint-btn-users");
    fireEvent.click(usersEndpointBtn);

    // Click Send Request
    const sendBtn = screen.getByTestId("btn-run-api-test");
    fireEvent.click(sendBtn);

    await waitFor(() => {
      expect(screen.getByTestId("api-test-response-console")).toBeInTheDocument();
      expect(screen.getByText(/HTTP 200 OK/i)).toBeInTheDocument();
      expect(screen.getByText(/users_count/i)).toBeInTheDocument();
    });
  });

  it("executes privileged endpoint outside token scope and receives 403 Forbidden (DoD)", async () => {
    render(
      <ToastProvider>
        <ApiDocsExplorer tokens={mockTokens} selectedTokenId="tok-test-01" />
      </ToastProvider>
    );

    // Select POST /admin/nodes (which requires cluster:admin, not granted on tok-test-01)
    const nodesEndpointBtn = screen.getByTestId("endpoint-btn-nodes");
    fireEvent.click(nodesEndpointBtn);

    // Click Send Request
    const sendBtn = screen.getByTestId("btn-run-api-test");
    fireEvent.click(sendBtn);

    await waitFor(() => {
      expect(screen.getByTestId("api-test-response-console")).toBeInTheDocument();
      expect(screen.getByText(/HTTP 403 Forbidden/i)).toBeInTheDocument();
      expect(screen.getByText(/lacks required scope/i)).toBeInTheDocument();
    });
  });
});
