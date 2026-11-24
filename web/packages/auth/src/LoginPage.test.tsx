import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";
import { LoginPage } from "./LoginPage";
import { SessionProvider } from "./context";
import { QueryClient, QueryClientProvider } from "@builtpanel/api";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderWithProviders(ui: React.ReactElement) {
  return render(
    <QueryClientProvider client={queryClient}>
      <SessionProvider bootstrapPath="/api/v1/admin/bootstrap">
        {ui}
      </SessionProvider>
    </QueryClientProvider>
  );
}

describe("LoginPage component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders username, password inputs and submit button", () => {
    renderWithProviders(<LoginPage title="Built Panel Admin" />);

    expect(screen.getByText("Built Panel Admin")).toBeInTheDocument();
    expect(screen.getByLabelText(/ইউজারনেম অথবা ইমেইল|Username or Email/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/পাসওয়ার্ড|Password/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /লগইন করুন|Sign In/i })).toBeInTheDocument();
  });

  it("shows error validation when submitting empty fields", async () => {
    renderWithProviders(<LoginPage />);

    const submitBtn = screen.getByRole("button", { name: /লগইন করুন|Sign In/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });

  it("shows error message on invalid credentials", async () => {
    renderWithProviders(<LoginPage />);

    const usernameInput = screen.getByLabelText(/ইউজারনেম অথবা ইমেইল|Username or Email/i);
    const passwordInput = screen.getByLabelText(/পাসওয়ার্ড|Password/i);
    const submitBtn = screen.getByRole("button", { name: /লগইন করুন|Sign In/i });

    fireEvent.change(usernameInput, { target: { value: "invalid" } });
    fireEvent.change(passwordInput, { target: { value: "wrong" } });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });

  it("switches to 2FA step when account requires TOTP verification", async () => {
    renderWithProviders(<LoginPage />);

    const usernameInput = screen.getByLabelText(/ইউজারনেম অথবা ইমেইল|Username or Email/i);
    const passwordInput = screen.getByLabelText(/পাসওয়ার্ড|Password/i);
    const submitBtn = screen.getByRole("button", { name: /লগইন করুন|Sign In/i });

    fireEvent.change(usernameInput, { target: { value: "admin_2fa" } });
    fireEvent.change(passwordInput, { target: { value: "correct_password" } });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(
        screen.getByLabelText(/টু-ফ্যাক্টর অথেনটিকেশন|Two-Factor Authentication/i)
      ).toBeInTheDocument();
    });
  });

  it("calls onSuccess upon successful login", async () => {
    const onSuccess = vi.fn();
    renderWithProviders(<LoginPage onSuccess={onSuccess} />);

    const usernameInput = screen.getByLabelText(/ইউজারনেম অথবা ইমেইল|Username or Email/i);
    const passwordInput = screen.getByLabelText(/পাসওয়ার্ড|Password/i);
    const submitBtn = screen.getByRole("button", { name: /লগইন করুন|Sign In/i });

    fireEvent.change(usernameInput, { target: { value: "admin" } });
    fireEvent.change(passwordInput, { target: { value: "valid_password_123" } });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalled();
    });
  });
});
