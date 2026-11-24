import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { DummyModulePage } from "./DummyModulePage";

let mockSession = {
  modules: ["dummy"],
  features: ["dummy_feature"],
};

vi.mock("@builtpanel/auth", () => ({
  useSession: () => ({
    data: mockSession,
    isLoading: false,
    hasFeature: () => true,
    hasPermission: () => true,
    hasRole: () => true,
  }),
}));

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const dict: Record<string, string> = {
        "dummy.title": "Dummy Module (T03.11)",
        "dummy.desc": "Runtime modularity verification.",
        "dummy.badge": "Modular Plugin",
        "dummy.status": "Active and Verified",
        "dummy.activeNotice": "This page is active.",
        "common.featureDisabled": "This feature is not enabled for your plan",
        "common.backHome": "Back to Dashboard",
      };
      return dict[key] || key;
    },
  }),
}));

describe("DummyModulePage (Feature-Gate & Plan Verification)", () => {
  beforeEach(() => {
    mockSession = {
      modules: ["dummy"],
      features: ["dummy_feature"],
    };
  });

  it("renders module content when module and feature are enabled", () => {
    mockSession = {
      modules: ["dummy"],
      features: ["dummy_feature"],
    };
    render(<DummyModulePage />);

    expect(screen.getByText("Dummy Module (T03.11)")).toBeInTheDocument();
    expect(screen.getByText("Modular Plugin")).toBeInTheDocument();
    expect(screen.getByText("Active and Verified")).toBeInTheDocument();

    const actionBtn = screen.getByRole("button", { name: /Interactive Action/i });
    expect(actionBtn).toBeInTheDocument();
    fireEvent.click(actionBtn);
    expect(screen.getByRole("button", { name: /Interactive Action \(1\)/i })).toBeInTheDocument();
  });

  it("renders Plan-এ নেই fallback when feature or module is disabled in plan", () => {
    mockSession = {
      modules: ["dashboard"], // "dummy" not enabled
      features: ["dashboard"],
    };
    render(<DummyModulePage />);

    expect(screen.getByText("This feature is not enabled for your plan")).toBeInTheDocument();
    expect(screen.getByText(/dummy_feature/)).toBeInTheDocument();
    expect(screen.queryByText("Modular Plugin")).not.toBeInTheDocument();
  });
});
