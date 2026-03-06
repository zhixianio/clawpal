import { describe, expect, test } from "bun:test";
import {
  getQuickDiagnoseTransport,
  buildPrefillMessage,
  shouldSeedContext,
  handleQuickDiagnoseDialogOpenChange,
} from "../quick-diagnose-utils";

describe("getQuickDiagnoseTransport", () => {
  test("returns remote_ssh when isRemote=true", () => {
    expect(getQuickDiagnoseTransport(true, false)).toBe("remote_ssh");
  });

  test("returns remote_ssh even when both true (isRemote wins)", () => {
    expect(getQuickDiagnoseTransport(true, true)).toBe("remote_ssh");
  });

  test("returns docker_local when isRemote=false isDocker=true", () => {
    expect(getQuickDiagnoseTransport(false, true)).toBe("docker_local");
  });

  test("returns local when both false", () => {
    expect(getQuickDiagnoseTransport(false, false)).toBe("local");
  });
});

describe("buildPrefillMessage", () => {
  test("trims whitespace", () => {
    expect(buildPrefillMessage("  hello  ")).toBe("hello");
  });

  test("returns empty string for null", () => {
    expect(buildPrefillMessage(null)).toBe("");
  });

  test("returns empty string for undefined", () => {
    expect(buildPrefillMessage(undefined)).toBe("");
  });

  test("returns empty string for empty string", () => {
    expect(buildPrefillMessage("")).toBe("");
  });

  test("preserves inner spaces", () => {
    expect(buildPrefillMessage("  error: host not found  ")).toBe("error: host not found");
  });
});

describe("shouldSeedContext", () => {
  test("true when context is non-empty and not yet seeded", () => {
    expect(shouldSeedContext("connection timeout", "")).toBe(true);
  });

  test("false when context is empty string", () => {
    expect(shouldSeedContext("", "")).toBe(false);
  });

  test("false when context is null", () => {
    expect(shouldSeedContext(null, "")).toBe(false);
  });

  test("false when context is undefined", () => {
    expect(shouldSeedContext(undefined, "")).toBe(false);
  });

  test("false when already seeded with same value", () => {
    expect(shouldSeedContext("connection timeout", "connection timeout")).toBe(false);
  });

  test("true when context differs from previously seeded", () => {
    expect(shouldSeedContext("new error", "old error")).toBe(true);
  });

  test("whitespace-only context treated as empty", () => {
    expect(shouldSeedContext("   ", "")).toBe(false);
  });
});

describe("handleQuickDiagnoseDialogOpenChange", () => {
  test("calls onOpenChange with false", () => {
    const calls: boolean[] = [];
    handleQuickDiagnoseDialogOpenChange((v) => calls.push(v), false);
    expect(calls).toEqual([false]);
  });

  test("calls onOpenChange with true", () => {
    const calls: boolean[] = [];
    handleQuickDiagnoseDialogOpenChange((v) => calls.push(v), true);
    expect(calls).toEqual([true]);
  });

  test("calls onOpenChange exactly once", () => {
    let count = 0;
    handleQuickDiagnoseDialogOpenChange(() => count++, true);
    expect(count).toBe(1);
  });
});
