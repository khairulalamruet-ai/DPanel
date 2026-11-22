import { describe, it, expect } from "vitest";
import { getEnabledModules, filterNavItems, registeredFrontendModules } from "./registry";

describe("Module Registry (T03.11)", () => {
  it("includes core modules by default", () => {
    const enabled = getEnabledModules([]);
    const coreNames = enabled.map((m) => m.name);
    expect(coreNames).toContain("dashboard");
    expect(coreNames).toContain("users");
    expect(coreNames).toContain("settings");
    expect(coreNames).not.toContain("domains");
    expect(coreNames).not.toContain("dummy");
  });

  it("enables non-core modules when requested in activeModuleNames", () => {
    const enabled = getEnabledModules(["domains", "dummy"]);
    const names = enabled.map((m) => m.name);
    expect(names).toContain("domains");
    expect(names).toContain("dummy");
    expect(names).toContain("dashboard");
  });

  describe("filterNavItems (DoD: menu changes when module is enabled/disabled)", () => {
    const baseSession = {
      roles: ["super_admin"],
      permissions: ["*"],
      modules: ["dashboard", "security"],
      features: ["all"],
    };

    it("does not include dummy module in navigation when dummy is disabled", () => {
      const nav = filterNavItems(registeredFrontendModules, baseSession, "admin");
      const navIds = nav.map((item) => item.id);
      expect(navIds).toContain("dashboard");
      expect(navIds).toContain("security");
      expect(navIds).not.toContain("dummy");
    });

    it("includes dummy module in navigation when dummy is enabled", () => {
      const sessionWithDummy = {
        ...baseSession,
        modules: ["dashboard", "security", "dummy"],
        features: ["all"],
      };

      const nav = filterNavItems(registeredFrontendModules, sessionWithDummy, "admin");
      const navIds = nav.map((item) => item.id);
      expect(navIds).toContain("dashboard");
      expect(navIds).toContain("security");
      expect(navIds).toContain("dummy");
    });

    it("filters out nav items when plan feature is not granted", () => {
      const sessionWithoutFeature = {
        roles: ["super_admin"],
        permissions: ["*"],
        modules: ["dashboard", "security", "dummy"],
        features: ["dashboard", "security"], // "dummy_feature" missing
      };

      const nav = filterNavItems(registeredFrontendModules, sessionWithoutFeature, "admin");
      const navIds = nav.map((item) => item.id);
      expect(navIds).not.toContain("dummy");
    });

    it("filters out nav items when permission is not granted", () => {
      const sessionWithoutPermission = {
        roles: ["client"],
        permissions: ["dashboard.read"], // "dummy.read" missing
        modules: ["dashboard", "security", "dummy"],
        features: ["all"],
      };

      const nav = filterNavItems(registeredFrontendModules, sessionWithoutPermission, "admin");
      const navIds = nav.map((item) => item.id);
      expect(navIds).not.toContain("dummy");
    });
  });
});

describe("Module registry has no duplicates", () => {
  it("lists every module name and every admin menu entry once", () => {
    const names = registeredFrontendModules.map((m) => m.manifest.name);
    expect(names.filter((n, i) => names.indexOf(n) !== i)).toEqual([]);
    for (const nav of ["adminNav", "clientNav"] as const) {
      const ids = registeredFrontendModules.flatMap((m) => (m[nav] ?? []).map((i) => i.id));
      expect(ids.filter((n, i) => ids.indexOf(n) !== i)).toEqual([]);
      const hrefs = registeredFrontendModules.flatMap((m) => (m[nav] ?? []).map((i) => i.href));
      expect(hrefs.filter((n, i) => hrefs.indexOf(n) !== i)).toEqual([]);
    }
  });
});
