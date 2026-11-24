import { describe, it, expect } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { HealthCard } from "./HealthCard";

describe("HealthCard component", () => {
  it("renders health status from API mock", async () => {
    render(<HealthCard />);
    await waitFor(() => {
      expect(screen.getByText(/0.1.0/i)).toBeInTheDocument();
    });
    // The badge ("OK") and the status line ("ok") both show it, now that the default language
    // is English.
    expect(screen.getAllByText(/^ok$/i).length).toBeGreaterThanOrEqual(1);
  });
});
