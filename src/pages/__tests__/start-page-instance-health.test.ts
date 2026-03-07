import { describe, expect, test } from "bun:test";

import { shouldShowLocalNotInstalled } from "../start-page-instance-health";

describe("shouldShowLocalNotInstalled", () => {
  test("stays false before local health has resolved", () => {
    expect(shouldShowLocalNotInstalled("local", undefined)).toBe(false);
  });

  test("treats a resolved unhealthy local instance as not installed", () => {
    expect(shouldShowLocalNotInstalled("local", { healthy: false, agentCount: 1 })).toBe(true);
  });

  test("treats a resolved unknown local instance as not installed", () => {
    expect(shouldShowLocalNotInstalled("local", { healthy: null, agentCount: 0 })).toBe(true);
  });

  test("keeps healthy local and non-local instances on the normal path", () => {
    expect(shouldShowLocalNotInstalled("local", { healthy: true, agentCount: 2 })).toBe(false);
    expect(shouldShowLocalNotInstalled("ssh:hetzner", { healthy: false, agentCount: 4 })).toBe(false);
  });
});
