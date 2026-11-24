import { describe, it, expect, vi } from "vitest";
import React from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { FeatureRequiredPage } from "./FeatureRequiredPage";

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const dict: Record<string, string> = {
        "common.featureDisabled": "This feature is not enabled for your plan",
        "common.featureDisabledDesc": "Upgrade your plan to access this feature.",
        "common.backHome": "Back to Dashboard",
        "common.upgradePlan": "Upgrade Plan",
      };
      return dict[key] || key;
    },
  }),
}));

describe("FeatureRequiredPage (Plan-এ নেই)", () => {
  it("renders feature required message and upgrade option", () => {
    render(<FeatureRequiredPage feature="advanced_dns" />);

    expect(screen.getByText("This feature is not enabled for your plan")).toBeInTheDocument();
    expect(screen.getByText(/advanced_dns/)).toBeInTheDocument();
    expect(screen.getByText("Back to Dashboard")).toBeInTheDocument();
    expect(screen.getByText("Upgrade Plan")).toBeInTheDocument();
  });

  it("triggers onNavigateHome callback when clicking back to dashboard", () => {
    const onNavigateHome = vi.fn();
    render(<FeatureRequiredPage onNavigateHome={onNavigateHome} />);

    const backBtn = screen.getByRole("button", { name: /Back to Dashboard/i });
    fireEvent.click(backBtn);

    expect(onNavigateHome).toHaveBeenCalledTimes(1);
  });
});
