import { describe, expect, test, spyOn, beforeEach, afterEach } from "bun:test";

import {
  isSshCooldownProtectionError,
  isTransientSshChannelError,
  isAlreadyExplainedGuidanceError,
  isRegistryCorruptError,
  isContainerOrphanedError,
  shouldEmitAgentGuidance,
  explainAndBuildGuidanceError,
  withGuidance,
} from "../guidance";
import { api } from "../api";

describe("isSshCooldownProtectionError", () => {
  test("matches ssh_cooldown prefix", () => {
    expect(isSshCooldownProtectionError("SSH_COOLDOWN: wait 30s")).toBe(true);
  });

  test("matches cooling down text", () => {
    expect(isSshCooldownProtectionError("connections are cooling down after repeated timeouts")).toBe(true);
  });

  test("matches retry in", () => {
    expect(isSshCooldownProtectionError("retry in 15 seconds")).toBe(true);
  });

  test("matches Chinese text", () => {
    expect(isSshCooldownProtectionError("处于冷却期，请稍后")).toBe(true);
    expect(isSshCooldownProtectionError("多次超时后暂停")).toBe(true);
  });

  test("does not match unrelated errors", () => {
    expect(isSshCooldownProtectionError("connection refused")).toBe(false);
    expect(isSshCooldownProtectionError("permission denied")).toBe(false);
  });
});

describe("isTransientSshChannelError", () => {
  test("matches open channel failed", () => {
    expect(isTransientSshChannelError("ssh open channel failed: timeout")).toBe(true);
  });

  test("matches connection reset", () => {
    expect(isTransientSshChannelError("connection reset by peer")).toBe(true);
  });

  test("matches broken pipe", () => {
    expect(isTransientSshChannelError("write failed: broken pipe")).toBe(true);
  });

  test("matches connection closed", () => {
    expect(isTransientSshChannelError("connection closed unexpectedly")).toBe(true);
  });

  test("matches failed to open channel", () => {
    expect(isTransientSshChannelError("failed to open channel")).toBe(true);
  });

  test("does not match auth errors", () => {
    expect(isTransientSshChannelError("authentication failed")).toBe(false);
  });
});

describe("isAlreadyExplainedGuidanceError", () => {
  test("matches Chinese guidance text", () => {
    expect(isAlreadyExplainedGuidanceError("下一步建议：执行诊断")).toBe(true);
    expect(isAlreadyExplainedGuidanceError("建议先做诊断再继续")).toBe(true);
    expect(isAlreadyExplainedGuidanceError("建议先执行诊断命令")).toBe(true);
    expect(isAlreadyExplainedGuidanceError("本机未安装 openclaw")).toBe(true);
  });

  test("matches English guidance text", () => {
    expect(isAlreadyExplainedGuidanceError("We recommend running doctor first")).toBe(true);
    expect(isAlreadyExplainedGuidanceError("next step: check config")).toBe(true);
    expect(isAlreadyExplainedGuidanceError("Please open doctor to diagnose")).toBe(true);
  });

  test("does not match raw errors", () => {
    expect(isAlreadyExplainedGuidanceError("ECONNREFUSED 127.0.0.1:22")).toBe(false);
  });
});

describe("isRegistryCorruptError", () => {
  test("matches registry parse error", () => {
    expect(isRegistryCorruptError("failed to parse registry")).toBe(true);
  });

  test("matches instances.json corrupt", () => {
    expect(isRegistryCorruptError("instances.json is corrupt")).toBe(true);
  });

  test("matches invalid json in registry", () => {
    expect(isRegistryCorruptError("registry: invalid json at line 5")).toBe(true);
  });

  test("does not match unrelated", () => {
    expect(isRegistryCorruptError("file not found")).toBe(false);
    expect(isRegistryCorruptError("registry updated successfully")).toBe(false);
  });
});

describe("isContainerOrphanedError", () => {
  test("matches no such container", () => {
    expect(isContainerOrphanedError("No such container: abc123")).toBe(true);
  });

  test("matches container not found", () => {
    expect(isContainerOrphanedError("container xyz not found")).toBe(true);
  });

  test("does not match openclaw container messages", () => {
    expect(isContainerOrphanedError("container openclaw not found")).toBe(false);
  });

  test("does not match unrelated", () => {
    expect(isContainerOrphanedError("image pull failed")).toBe(false);
  });
});

describe("shouldEmitAgentGuidance", () => {
  test("suppresses cooldown errors", () => {
    expect(shouldEmitAgentGuidance("inst1", "connect", "SSH_COOLDOWN: wait")).toBe(false);
  });

  test("suppresses transient channel errors", () => {
    expect(shouldEmitAgentGuidance("inst1", "exec", "ssh open channel failed")).toBe(false);
  });

  test("suppresses already-explained errors", () => {
    expect(shouldEmitAgentGuidance("inst1", "health", "建议先做诊断再继续")).toBe(false);
  });

  test("allows novel errors", () => {
    const unique = `unique-error-${Date.now()}-${Math.random()}`;
    expect(shouldEmitAgentGuidance("inst1", "connect", unique)).toBe(true);
  });

  test("throttles duplicate errors within 90s", () => {
    const unique = `throttle-test-${Date.now()}-${Math.random()}`;
    expect(shouldEmitAgentGuidance("inst-throttle", "op", unique)).toBe(true);
    expect(shouldEmitAgentGuidance("inst-throttle", "op", unique)).toBe(false);
  });

  test("different instances are independent", () => {
    const unique = `multi-inst-${Date.now()}-${Math.random()}`;
    expect(shouldEmitAgentGuidance("instA", "op", unique)).toBe(true);
    expect(shouldEmitAgentGuidance("instB", "op", unique)).toBe(true);
  });

  test("cleanup fires when map exceeds 256 entries", () => {
    // Fill the throttle map with >256 unique entries
    const now = Date.now();
    for (let i = 0; i < 260; i++) {
      const uniqueErr = `cleanup-fill-${now}-${i}-${Math.random()}`;
      shouldEmitAgentGuidance("cleanup-inst", `op-${i}`, uniqueErr);
    }
    // The 257th+ insertions trigger the cleanup path (lines 98-103).
    // We just need this to run without errors; coverage is the goal.
    const finalErr = `cleanup-final-${now}-${Math.random()}`;
    expect(shouldEmitAgentGuidance("cleanup-inst", "final", finalErr)).toBe(true);
  });
});

// ── Async function tests ──

describe("explainAndBuildGuidanceError", () => {
  let explainSpy: ReturnType<typeof spyOn>;
  let dispatchSpy: ReturnType<typeof spyOn> | undefined;
  let originalWindow: typeof globalThis.window;
  let originalCustomEvent: typeof globalThis.CustomEvent;

  beforeEach(() => {
    explainSpy = spyOn(api, "explainOperationError");
    originalWindow = globalThis.window;
    originalCustomEvent = globalThis.CustomEvent;
    const existingWindow =
      typeof globalThis.window === "object" && globalThis.window !== null
        ? (globalThis.window as unknown as Record<string, unknown>)
        : {};
    (globalThis as any).window = {
      ...existingWindow,
      dispatchEvent:
        typeof existingWindow.dispatchEvent === "function"
          ? existingWindow.dispatchEvent
          : () => true,
    };
    if (typeof globalThis.CustomEvent === "undefined") {
      (globalThis as any).CustomEvent = class CustomEvent extends Event {
        detail: any;
        constructor(type: string, init?: { detail?: any }) {
          super(type);
          this.detail = init?.detail;
        }
      };
    }
    dispatchSpy = spyOn(globalThis.window, "dispatchEvent");
  });

  afterEach(() => {
    explainSpy.mockRestore();
    dispatchSpy?.mockRestore();
    if (typeof originalWindow === "undefined") {
      delete (globalThis as any).window;
    } else {
      (globalThis as any).window = originalWindow;
    }
    if (typeof originalCustomEvent === "undefined") {
      delete (globalThis as any).CustomEvent;
    } else {
      (globalThis as any).CustomEvent = originalCustomEvent;
    }
  });

  test("returns original error for cooldown errors without calling API", async () => {
    const result = await explainAndBuildGuidanceError({
      method: "connect",
      instanceId: "inst1",
      transport: "remote_ssh",
      rawError: "SSH_COOLDOWN: wait 30s",
    });
    expect(result).toBeInstanceOf(Error);
    expect(result.message).toContain("SSH_COOLDOWN");
    expect(explainSpy).not.toHaveBeenCalled();
  });

  test("returns original error for transient channel errors", async () => {
    const result = await explainAndBuildGuidanceError({
      method: "exec",
      instanceId: "inst1",
      transport: "remote_ssh",
      rawError: "ssh open channel failed: timeout",
    });
    expect(result.message).toContain("ssh open channel failed");
    expect(explainSpy).not.toHaveBeenCalled();
  });

  test("returns original error for already-explained errors", async () => {
    const result = await explainAndBuildGuidanceError({
      method: "health",
      instanceId: "inst1",
      transport: "local",
      rawError: "建议先做诊断再继续",
    });
    expect(result.message).toContain("建议先做诊断再继续");
    expect(explainSpy).not.toHaveBeenCalled();
  });

  test("calls API and returns wrapped error on normal path", async () => {
    const uniqueErr = `normal-path-${Date.now()}-${Math.random()}`;
    explainSpy.mockResolvedValueOnce({
      message: "Explained: something went wrong",
      summary: "summary",
      actions: ["restart"],
      structuredActions: [],
      source: "zeroclaw",
    });

    const result = await explainAndBuildGuidanceError({
      method: "listAgents",
      instanceId: "explain-inst",
      transport: "local",
      rawError: uniqueErr,
    });

    expect(explainSpy).toHaveBeenCalledTimes(1);
    expect(result.message).toBe("Explained: something went wrong");
    // emitEvent defaults to true, so _guidanceEmitted should be set
    expect((result as any)._guidanceEmitted).toBe(true);
  });

  test("keeps explained error when window exists but cannot dispatch events", async () => {
    const uniqueErr = `window-stub-${Date.now()}-${Math.random()}`;
    (globalThis as any).window = {
      localStorage: {
        getItem: () => null,
      },
    };
    explainSpy.mockResolvedValueOnce({
      message: "Explained despite missing dispatchEvent",
      summary: "summary",
      actions: [],
      structuredActions: [],
      source: "zeroclaw",
    });

    const result = await explainAndBuildGuidanceError({
      method: "listAgents",
      instanceId: "stub-inst",
      transport: "local",
      rawError: uniqueErr,
      emitEvent: true,
    });

    expect(explainSpy).toHaveBeenCalledTimes(1);
    expect(result.message).toBe("Explained despite missing dispatchEvent");
    expect((result as any)._guidanceEmitted).toBeUndefined();
  });

  test("dispatches CustomEvent when emitEvent is true", async () => {
    const uniqueErr = `event-test-${Date.now()}-${Math.random()}`;
    explainSpy.mockResolvedValueOnce({
      message: "explained",
      summary: "s",
      actions: [],
      structuredActions: [],
      source: "test",
    });

    await explainAndBuildGuidanceError({
      method: "connect",
      instanceId: "event-inst",
      transport: "remote_ssh",
      rawError: uniqueErr,
      emitEvent: true,
    });

    expect(dispatchSpy).toHaveBeenCalled();
    // Find the clawpal:agent-guidance event
    const guidanceCalls = dispatchSpy!.mock.calls.filter(
      (call: any) => call[0]?.type === "clawpal:agent-guidance"
    );
    expect(guidanceCalls.length).toBeGreaterThanOrEqual(1);
    const detail = (guidanceCalls[0] as any)[0].detail;
    expect(detail.operation).toBe("connect");
    expect(detail.instanceId).toBe("event-inst");
  });

  test("does NOT dispatch event when emitEvent is false", async () => {
    const uniqueErr = `no-event-${Date.now()}-${Math.random()}`;
    explainSpy.mockResolvedValueOnce({
      message: "silent",
      summary: "",
      actions: [],
      structuredActions: [],
      source: "test",
    });
    dispatchSpy!.mockClear();

    const result = await explainAndBuildGuidanceError({
      method: "op",
      instanceId: "silent-inst",
      transport: "local",
      rawError: uniqueErr,
      emitEvent: false,
    });

    expect(result.message).toBe("silent");
    expect((result as any)._guidanceEmitted).toBeUndefined();
    // No guidance event should have been dispatched
    const guidanceCalls = dispatchSpy!.mock.calls.filter(
      (call: any) => call[0]?.type === "clawpal:agent-guidance"
    );
    expect(guidanceCalls.length).toBe(0);
  });

  test("falls back to original error when API throws", async () => {
    const uniqueErr = `api-fail-${Date.now()}-${Math.random()}`;
    explainSpy.mockRejectedValueOnce(new Error("network error"));

    const result = await explainAndBuildGuidanceError({
      method: "exec",
      instanceId: "fail-inst",
      transport: "remote_ssh",
      rawError: uniqueErr,
    });

    expect(result.message).toBe(uniqueErr);
    expect((result as any)._guidanceEmitted).toBeUndefined();
  });
});

describe("withGuidance", () => {
  let explainSpy: ReturnType<typeof spyOn>;

  beforeEach(() => {
    explainSpy = spyOn(api, "explainOperationError");
  });

  afterEach(() => {
    explainSpy.mockRestore();
  });

  test("returns result when fn succeeds", async () => {
    const result = await withGuidance(
      async () => "success-value",
      "listAgents",
      "inst1",
      "local",
    );
    expect(result).toBe("success-value");
    expect(explainSpy).not.toHaveBeenCalled();
  });

  test("throws wrapped error when fn fails", async () => {
    const uniqueErr = `with-guidance-fail-${Date.now()}-${Math.random()}`;
    explainSpy.mockResolvedValueOnce({
      message: "Guidance: check config",
      summary: "config issue",
      actions: [],
      structuredActions: [],
      source: "test",
    });

    try {
      await withGuidance(
        async () => { throw new Error(uniqueErr); },
        "readConfig",
        "wg-inst",
        "docker_local",
      );
      // Should not reach here
      expect(true).toBe(false);
    } catch (err: any) {
      expect(err).toBeInstanceOf(Error);
      expect(err.message).toBe("Guidance: check config");
      expect(explainSpy).toHaveBeenCalledTimes(1);
    }
  });

  test("throws original error when explain API fails", async () => {
    const uniqueErr = `wg-api-fail-${Date.now()}-${Math.random()}`;
    explainSpy.mockRejectedValueOnce(new Error("api down"));

    try {
      await withGuidance(
        // throw a string so String(rawError) === uniqueErr (no "Error: " prefix)
        async () => { throw uniqueErr; },
        "exec",
        "wg-inst2",
        "remote_ssh",
      );
      expect(true).toBe(false);
    } catch (err: any) {
      // Falls back to original error string
      expect(err.message).toBe(uniqueErr);
    }
  });
});
