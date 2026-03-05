import { describe, expect, test } from "bun:test";

import {
  closeWorkspaceTab,
  shouldRenderGuidanceCard,
} from "../tabWorkspace";

describe("closeWorkspaceTab", () => {
  test("closes inactive tab without changing active instance", () => {
    const result = closeWorkspaceTab({
      openTabIds: ["local", "docker:a", "ssh:b"],
      activeInstance: "docker:a",
      inStart: false,
      startSection: "overview",
    }, "ssh:b");

    expect(result.openTabIds).toEqual(["local", "docker:a"]);
    expect(result.activeInstance).toBe("docker:a");
    expect(result.inStart).toBe(false);
  });

  test("when closing active tab, switches to the last remaining tab", () => {
    const result = closeWorkspaceTab({
      openTabIds: ["local", "docker:a", "ssh:b"],
      activeInstance: "ssh:b",
      inStart: false,
      startSection: "overview",
    }, "ssh:b");

    expect(result.openTabIds).toEqual(["local", "docker:a"]);
    expect(result.activeInstance).toBe("docker:a");
    expect(result.inStart).toBe(false);
  });

  test("when closing last tab, enters start mode and resets section", () => {
    const result = closeWorkspaceTab({
      openTabIds: ["ssh:b"],
      activeInstance: "ssh:b",
      inStart: false,
      startSection: "profiles",
    }, "ssh:b");

    expect(result.openTabIds).toEqual([]);
    expect(result.inStart).toBe(true);
    expect(result.startSection).toBe("overview");
    expect(result.activeInstance).toBe("local");
  });
});

describe("shouldRenderGuidanceCard", () => {
  test("requires guidance payload even when panel is open", () => {
    expect(shouldRenderGuidanceCard(true, null)).toBe(false);
  });

  test("renders only when open and guidance exists", () => {
    expect(shouldRenderGuidanceCard(false, { instanceId: "local" })).toBe(false);
    expect(shouldRenderGuidanceCard(true, { instanceId: "local" })).toBe(true);
  });
});
