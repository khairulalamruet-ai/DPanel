import { describe, it, expect } from "vitest";
import { toLicense } from "./api";

describe("license adapter", () => {
  it("reads what the backend says about an active license", () => {
    const l = toLicense({
      enforced: true,
      mode: "active",
      plan: "Pro",
      installation_id: "inst-1",
      license_status: "active",
      license_expires_at: 1893456000,
      last_confirmed_at: 1790000000,
      features: ["reseller"],
      limits: { max_nodes: 3 },
    });
    expect(l.status).toBe("active");
    expect(l.plan_tier).toBe("pro");
    expect(l.features.reseller_enabled).toBe(true);
    expect(l.features.max_nodes).toBe(3);
    expect(l.features.max_tenants).toBe(-1);
    expect(l.expires_at.startsWith("2030-01-01")).toBe(true);
    expect(l.signature_valid).toBe(true);
  });

  it("shows the offline grace and the reason of a restriction", () => {
    const grace = toLicense({ enforced: true, mode: "grace", grace_ends_at: Math.floor(Date.now() / 1000) + 7200, features: [], limits: null });
    expect(grace.status).toBe("grace_period");
    expect(grace.offline_grace_hours_remaining).toBe(2);
    const restricted = toLicense({ enforced: true, mode: "restricted", reason: "license_revoked", features: [], limits: null, last_error: "revoked" });
    expect(restricted.status).toBe("restricted");
    expect(restricted.restricted_reason).toBe("license_revoked");
    expect(restricted.signature_valid).toBe(false);
  });
});
