import { describe, expect, test } from "bun:test";

import {
  isSshCooldownProtectionError,
  isTransientSshChannelError,
  isAlreadyExplainedGuidanceError,
  isRegistryCorruptError,
  isContainerOrphanedError,
  shouldEmitAgentGuidance,
} from "../guidance";

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
});
