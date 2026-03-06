import { describe, expect, test } from "bun:test";
import {
  normalizeErrorSignature,
  isSshCooldownProtectionError,
  isTransientSshChannelError,
  isAlreadyExplainedGuidanceError,
  explainAndBuildGuidanceError,
} from "../guidance";
import { hasGuidanceEmitted } from "../use-api";

describe("normalizeErrorSignature", () => {
  test("lowercases input", () => {
    expect(normalizeErrorSignature("ERROR")).toBe("error");
  });

  test("collapses whitespace", () => {
    expect(normalizeErrorSignature("error   at   line")).toBe("error at line");
  });

  test("replaces multi-digit numbers with #", () => {
    expect(normalizeErrorSignature("error at line 42")).toBe("error at line #");
  });

  test("does not replace single digit", () => {
    expect(normalizeErrorSignature("step 1 failed")).toBe("step 1 failed");
  });

  test("trims whitespace", () => {
    expect(normalizeErrorSignature("  error  ")).toBe("error");
  });

  test("truncates to 220 characters", () => {
    const long = "a".repeat(300);
    expect(normalizeErrorSignature(long).length).toBe(220);
  });

  test("handles empty string", () => {
    expect(normalizeErrorSignature("")).toBe("");
  });

  test("combines all normalizations", () => {
    const result = normalizeErrorSignature("  Connection TIMEOUT at port 8080  after  300ms  ");
    expect(result).toBe("connection timeout at port # after #ms");
  });
});

describe("explainAndBuildGuidanceError", () => {
  test("returns original for cooldown errors", async () => {
    const error = await explainAndBuildGuidanceError({
      method: "connect",
      instanceId: "inst1",
      transport: "remote_ssh",
      rawError: "SSH_COOLDOWN: wait 30s",
      emitEvent: false,
    });
    expect(error.message).toBe("SSH_COOLDOWN: wait 30s");
  });

  test("returns original for transient SSH errors", async () => {
    const error = await explainAndBuildGuidanceError({
      method: "exec",
      instanceId: "inst1",
      transport: "remote_ssh",
      rawError: "ssh open channel failed",
      emitEvent: false,
    });
    expect(error.message).toBe("ssh open channel failed");
  });

  test("returns original for already-explained errors", async () => {
    const error = await explainAndBuildGuidanceError({
      method: "health",
      instanceId: "inst1",
      transport: "local",
      rawError: "建议先做诊断再继续",
      emitEvent: false,
    });
    expect(error.message).toBe("建议先做诊断再继续");
  });

  test("returns Error object", async () => {
    const error = await explainAndBuildGuidanceError({
      method: "connect",
      instanceId: "inst1",
      transport: "local",
      rawError: "SSH_COOLDOWN: wait",
      emitEvent: false,
    });
    expect(error).toBeInstanceOf(Error);
  });

  test("converts non-string rawError to string", async () => {
    const error = await explainAndBuildGuidanceError({
      method: "connect",
      instanceId: "inst1",
      transport: "local",
      rawError: new Error("SSH_COOLDOWN: inner"),
      emitEvent: false,
    });
    expect(error.message).toContain("SSH_COOLDOWN: inner");
  });
});

describe("hasGuidanceEmitted", () => {
  test("returns false for null", () => {
    expect(hasGuidanceEmitted(null)).toBe(false);
  });

  test("returns false for undefined", () => {
    expect(hasGuidanceEmitted(undefined)).toBe(false);
  });

  test("returns false for plain Error", () => {
    expect(hasGuidanceEmitted(new Error("test"))).toBe(false);
  });

  test("returns false for string", () => {
    expect(hasGuidanceEmitted("error string")).toBe(false);
  });

  test("returns false for number", () => {
    expect(hasGuidanceEmitted(42)).toBe(false);
  });

  test("returns true when _guidanceEmitted flag is set", () => {
    const error = new Error("test");
    (error as any)._guidanceEmitted = true;
    expect(hasGuidanceEmitted(error)).toBe(true);
  });

  test("returns false when _guidanceEmitted is false", () => {
    const error = new Error("test");
    (error as any)._guidanceEmitted = false;
    expect(hasGuidanceEmitted(error)).toBe(false);
  });
});
