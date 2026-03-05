import { describe, expect, test } from "bun:test";

import {
  hasAnyPrefix,
  isDoctorAutoSafeInvoke,
  normalizeInvokeArgs,
} from "../use-doctor-agent";
import type { DoctorInvoke } from "../types";

function makeInvoke(
  command: string,
  args: string,
  overrides?: Partial<DoctorInvoke>,
): DoctorInvoke {
  return { id: "test-1", command, args: { args }, type: "read", ...overrides };
}

// ---------------------------------------------------------------------------
// normalizeInvokeArgs
// ---------------------------------------------------------------------------

describe("normalizeInvokeArgs", () => {
  test("trims and collapses whitespace", () => {
    const inv = makeInvoke("clawpal", "  doctor   probe-openclaw  ");
    expect(normalizeInvokeArgs(inv)).toBe("doctor probe-openclaw");
  });

  test("lowercases", () => {
    const inv = makeInvoke("clawpal", "Doctor File Read");
    expect(normalizeInvokeArgs(inv)).toBe("doctor file read");
  });

  test("returns empty string for missing args", () => {
    const inv: DoctorInvoke = { id: "x", command: "clawpal", args: {}, type: "read" };
    expect(normalizeInvokeArgs(inv)).toBe("");
  });
});

// ---------------------------------------------------------------------------
// hasAnyPrefix
// ---------------------------------------------------------------------------

describe("hasAnyPrefix", () => {
  test("matches exact value", () => {
    expect(hasAnyPrefix("doctor", ["doctor"])).toBe(true);
  });

  test("matches prefix followed by space", () => {
    expect(hasAnyPrefix("config get foo", ["config get"])).toBe(true);
  });

  test("rejects partial prefix without space separator", () => {
    expect(hasAnyPrefix("config getter", ["config get"])).toBe(false);
  });

  test("rejects unrelated value", () => {
    expect(hasAnyPrefix("something else", ["doctor"])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// isDoctorAutoSafeInvoke — read-only commands (should be auto-safe)
// ---------------------------------------------------------------------------

describe("isDoctorAutoSafeInvoke — allowed (read-only)", () => {
  const safeClawpalCommands = [
    "doctor probe-openclaw",
    "doctor file read",
    "doctor file read /some/path",
    "doctor config-read",
    "doctor sessions-read",
  ];

  for (const args of safeClawpalCommands) {
    test(`clawpal: "${args}" is safe`, () => {
      expect(isDoctorAutoSafeInvoke(makeInvoke("clawpal", args), "doctor")).toBe(true);
    });
  }

  const safeOpenclawCommands = [
    "--version",
    "doctor",
    "doctor list",
    "gateway status",
    "health",
    "config get",
    "config get some.key",
    "agents list",
    "memory status",
    "security audit",
  ];

  for (const args of safeOpenclawCommands) {
    test(`openclaw: "${args}" is safe`, () => {
      expect(isDoctorAutoSafeInvoke(makeInvoke("openclaw", args), "doctor")).toBe(true);
    });
  }
});

// ---------------------------------------------------------------------------
// isDoctorAutoSafeInvoke — write/delete commands (must NOT be auto-safe)
// ---------------------------------------------------------------------------

describe("isDoctorAutoSafeInvoke — blocked (mutative)", () => {
  const unsafeClawpalCommands = [
    "doctor fix-openclaw-path",
    "doctor file write",
    "doctor file write /some/path",
    "doctor config-upsert",
    "doctor config-delete",
    "doctor sessions-upsert",
    "doctor sessions-delete",
  ];

  for (const args of unsafeClawpalCommands) {
    test(`clawpal: "${args}" is NOT safe`, () => {
      expect(isDoctorAutoSafeInvoke(makeInvoke("clawpal", args), "doctor")).toBe(false);
    });
  }

  const unsafeOpenclawCommands = [
    "config set",
    "config set some.key value",
    "config delete",
    "config delete some.key",
    "config unset",
    "config unset some.key",
  ];

  for (const args of unsafeOpenclawCommands) {
    test(`openclaw: "${args}" is NOT safe`, () => {
      expect(isDoctorAutoSafeInvoke(makeInvoke("openclaw", args), "doctor")).toBe(false);
    });
  }
});

// ---------------------------------------------------------------------------
// isDoctorAutoSafeInvoke — domain guard
// ---------------------------------------------------------------------------

describe("isDoctorAutoSafeInvoke — domain guard", () => {
  test("install domain is never auto-safe", () => {
    expect(isDoctorAutoSafeInvoke(makeInvoke("clawpal", "doctor probe-openclaw"), "install")).toBe(false);
  });

  test("unknown command is not auto-safe", () => {
    expect(isDoctorAutoSafeInvoke(makeInvoke("unknown", "anything"), "doctor")).toBe(false);
  });
});
