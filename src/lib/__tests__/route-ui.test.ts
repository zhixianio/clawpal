import { describe, expect, test } from "bun:test";

import { shouldShowPendingChangesBar } from "../route-ui";

describe("route-ui", () => {
  test("hides pending changes bar on cook route", () => {
    expect(shouldShowPendingChangesBar({ inStart: false, route: "cook" })).toBe(false);
  });

  test("shows pending changes bar on recipes route", () => {
    expect(shouldShowPendingChangesBar({ inStart: false, route: "recipes" })).toBe(true);
  });

  test("hides pending changes bar in start mode", () => {
    expect(shouldShowPendingChangesBar({ inStart: true, route: "recipes" })).toBe(false);
  });
});
