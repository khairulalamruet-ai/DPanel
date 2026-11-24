import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockServices } from "@builtpanel/api";
import { ServicesPage } from "./ServicesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ServicesPage (T06.12)", () => {
  beforeEach(() => {
    resetMockServices();
  });

  it("renders page title, policy notice, and service cards", async () => {
    renderWithProviders(<ServicesPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("service-card-nginx")).toBeInTheDocument();
      expect(screen.getByTestId("service-card-php-fpm")).toBeInTheDocument();
    });

    expect(screen.getByText("Nginx Web Server")).toBeInTheDocument();
    expect(screen.getByText("PHP-FPM Process Manager")).toBeInTheDocument();
  });

  it("displays service metrics and status badges", async () => {
    renderWithProviders(<ServicesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("badge-status-nginx")).toBeInTheDocument();
    });

    expect(screen.getByTestId("badge-status-nginx")).toHaveTextContent("RUNNING");
    expect(screen.getByTestId("badge-status-php-fpm")).toHaveTextContent("RUNNING");

    expect(screen.getByTestId("quota-restarts-nginx")).toHaveTextContent("0 / 3 restarts this minute");
  });

  it("restarts nginx service successfully and updates count", async () => {
    renderWithProviders(<ServicesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-restart-nginx")).toBeInTheDocument();
    });

    // 1st restart
    fireEvent.click(screen.getByTestId("btn-restart-nginx"));

    await waitFor(() => {
      expect(screen.getByTestId("quota-restarts-nginx")).toHaveTextContent("1 / 3 restarts this minute");
    });
  });

  it("DoD: triggers HTTP 429 rate limit when restarting more than 3 times in a minute", async () => {
    renderWithProviders(<ServicesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-restart-nginx")).toBeInTheDocument();
    });

    const restartBtn = screen.getByTestId("btn-restart-nginx");

    // 1st restart
    fireEvent.click(restartBtn);
    await waitFor(() => {
      expect(screen.getByTestId("quota-restarts-nginx")).toHaveTextContent("1 / 3 restarts this minute");
    });

    // 2nd restart
    fireEvent.click(restartBtn);
    await waitFor(() => {
      expect(screen.getByTestId("quota-restarts-nginx")).toHaveTextContent("2 / 3 restarts this minute");
    });

    // 3rd restart (maximum allowed)
    fireEvent.click(restartBtn);
    await waitFor(() => {
      expect(screen.getByTestId("quota-restarts-nginx")).toHaveTextContent("3 / 3 restarts this minute");
    });

    // 4th restart attempt within same minute -> must trigger 429 Rate Limit error!
    fireEvent.click(restartBtn);

    await waitFor(() => {
      expect(screen.getByTestId("banner-rate-limit-nginx")).toBeInTheDocument();
    });
    expect(screen.getByTestId("banner-rate-limit-nginx")).toHaveTextContent(
      /Rate limit exceeded|You can restart this service at most 3 times per minute/i
    );
  });
});
