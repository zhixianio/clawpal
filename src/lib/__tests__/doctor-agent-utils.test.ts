import { describe, expect, test } from "bun:test";
import {
  extractApprovalPattern,
  normalizeInvokeArgs,
  hasAnyPrefix,
  buildDoctorCacheKey,
  sanitizeDoctorCacheMessages,
  extractOpenclawText,
  extractOpenclawSessionId,
  isDoctorAutoSafeInvoke,
} from "../doctor-agent-utils";
import type { DoctorInvoke } from "../types";

function makeInvoke(overrides: Partial<DoctorInvoke> = {}): DoctorInvoke {
  return {
    id: "inv-1",
    command: "clawpal",
    args: {},
    type: "read",
    ...overrides,
  };
}

describe("extractApprovalPattern", () => {
  test("extracts command:prefix pattern with path", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { path: "/etc/config/file.toml" } });
    expect(extractApprovalPattern(invoke)).toBe("clawpal:/etc/config/");
  });

  test("uses full path when no slash", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { path: "config.toml" } });
    expect(extractApprovalPattern(invoke)).toBe("openclaw:config.toml");
  });

  test("handles missing path", () => {
    const invoke = makeInvoke({ command: "clawpal", args: {} });
    expect(extractApprovalPattern(invoke)).toBe("clawpal:");
  });

  test("handles root path", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { path: "/file.txt" } });
    expect(extractApprovalPattern(invoke)).toBe("clawpal:/");
  });

  test("handles deeply nested path", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { path: "/a/b/c/d.txt" } });
    expect(extractApprovalPattern(invoke)).toBe("clawpal:/a/b/c/");
  });
});

describe("normalizeInvokeArgs", () => {
  test("trims whitespace", () => {
    const invoke = makeInvoke({ args: { args: "  hello  " } });
    expect(normalizeInvokeArgs(invoke)).toBe("hello");
  });

  test("collapses multiple spaces", () => {
    const invoke = makeInvoke({ args: { args: "doctor   file   read" } });
    expect(normalizeInvokeArgs(invoke)).toBe("doctor file read");
  });

  test("lowercases everything", () => {
    const invoke = makeInvoke({ args: { args: "Doctor File READ" } });
    expect(normalizeInvokeArgs(invoke)).toBe("doctor file read");
  });

  test("handles missing args", () => {
    const invoke = makeInvoke({ args: {} });
    expect(normalizeInvokeArgs(invoke)).toBe("");
  });

  test("handles tabs and newlines", () => {
    const invoke = makeInvoke({ args: { args: "config\tget\nkey" } });
    expect(normalizeInvokeArgs(invoke)).toBe("config get key");
  });
});

describe("hasAnyPrefix", () => {
  test("matches exact value", () => {
    expect(hasAnyPrefix("doctor", ["doctor", "health"])).toBe(true);
  });

  test("matches prefix followed by space", () => {
    expect(hasAnyPrefix("doctor run", ["doctor"])).toBe(true);
  });

  test("does not match partial prefix without space", () => {
    expect(hasAnyPrefix("doctorx", ["doctor"])).toBe(false);
  });

  test("returns false when no match", () => {
    expect(hasAnyPrefix("config", ["doctor", "health"])).toBe(false);
  });

  test("handles empty prefixes array", () => {
    expect(hasAnyPrefix("anything", [])).toBe(false);
  });

  test("handles empty value", () => {
    expect(hasAnyPrefix("", ["doctor"])).toBe(false);
  });
});

describe("buildDoctorCacheKey", () => {
  test("builds correct key format", () => {
    const key = buildDoctorCacheKey({
      instanceScope: "local",
      agentId: "main",
      domain: "doctor",
      engine: "zeroclaw",
    });
    expect(key).toBe("clawpal-doctor-chat-v1-doctor-zeroclaw-local-main");
  });

  test("encodes special characters in scope and agent", () => {
    const key = buildDoctorCacheKey({
      instanceScope: "host/with spaces",
      agentId: "agent@1",
      domain: "doctor",
      engine: "openclaw",
    });
    expect(key).toContain("host%2Fwith%20spaces");
    expect(key).toContain("agent%401");
  });

  test("uses install domain", () => {
    const key = buildDoctorCacheKey({
      instanceScope: "local",
      agentId: "main",
      domain: "install",
      engine: "zeroclaw",
    });
    expect(key).toContain("-install-");
  });

  test("uses openclaw engine", () => {
    const key = buildDoctorCacheKey({
      instanceScope: "local",
      agentId: "main",
      domain: "doctor",
      engine: "openclaw",
    });
    expect(key).toContain("-openclaw-");
  });
});

describe("sanitizeDoctorCacheMessages", () => {
  test("returns [] for non-array input", () => {
    expect(sanitizeDoctorCacheMessages(null)).toEqual([]);
    expect(sanitizeDoctorCacheMessages(undefined)).toEqual([]);
    expect(sanitizeDoctorCacheMessages("string")).toEqual([]);
    expect(sanitizeDoctorCacheMessages(42)).toEqual([]);
    expect(sanitizeDoctorCacheMessages({})).toEqual([]);
  });

  test("filters out non-object entries", () => {
    expect(sanitizeDoctorCacheMessages([null, undefined, "str", 42])).toEqual([]);
  });

  test("filters out entries with invalid role", () => {
    expect(sanitizeDoctorCacheMessages([{ id: "1", role: "system", content: "hi" }])).toEqual([]);
  });

  test("filters out entries without id", () => {
    expect(sanitizeDoctorCacheMessages([{ role: "assistant", content: "hi" }])).toEqual([]);
    expect(sanitizeDoctorCacheMessages([{ id: "", role: "assistant", content: "hi" }])).toEqual([]);
  });

  test("preserves valid assistant message", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m1", role: "assistant", content: "hello" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({ id: "m1", role: "assistant", content: "hello" });
  });

  test("preserves valid user message", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m2", role: "user", content: "hi there" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0].role).toBe("user");
  });

  test("preserves tool-call message", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m3", role: "tool-call", content: "cmd" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0].role).toBe("tool-call");
  });

  test("preserves tool-result message", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m4", role: "tool-result", content: "ok" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0].role).toBe("tool-result");
  });

  test("preserves invoke field when present", () => {
    const invoke = { id: "inv1", command: "clawpal", args: { path: "/etc" }, type: "read" };
    const result = sanitizeDoctorCacheMessages([
      { id: "m5", role: "tool-call", content: "cmd", invoke },
    ]);
    expect(result[0].invoke).toEqual(invoke);
  });

  test("preserves invokeId field", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m6", role: "tool-result", content: "ok", invokeId: "inv1" },
    ]);
    expect(result[0].invokeId).toBe("inv1");
  });

  test("preserves invokeResult field", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m7", role: "tool-result", content: "ok", invokeResult: { data: "value" } },
    ]);
    expect(result[0].invokeResult).toEqual({ data: "value" });
  });

  test("preserves status field for valid statuses", () => {
    for (const status of ["pending", "approved", "rejected", "auto"] as const) {
      const result = sanitizeDoctorCacheMessages([
        { id: `m-${status}`, role: "tool-call", content: "cmd", status },
      ]);
      expect(result[0].status).toBe(status);
    }
  });

  test("ignores invalid status", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m8", role: "tool-call", content: "cmd", status: "invalid" },
    ]);
    expect(result[0].status).toBeUndefined();
  });

  test("preserves diagnosisReport with items array", () => {
    const report = { items: [{ problem: "test", severity: "error", fix_options: ["fix1"] }] };
    const result = sanitizeDoctorCacheMessages([
      { id: "m9", role: "assistant", content: "analysis", diagnosisReport: report },
    ]);
    expect(result[0].diagnosisReport?.items).toHaveLength(1);
    expect(result[0].diagnosisReport?.items[0].problem).toBe("test");
  });

  test("defaults content to empty string for non-string content", () => {
    const result = sanitizeDoctorCacheMessages([
      { id: "m10", role: "assistant", content: 42 },
    ]);
    expect(result[0].content).toBe("");
  });

  test("handles mixed valid and invalid messages", () => {
    const messages = [
      { id: "good1", role: "assistant", content: "hello" },
      null,
      { role: "assistant", content: "no id" },
      { id: "good2", role: "user", content: "world" },
      { id: "bad", role: "system", content: "invalid role" },
    ];
    const result = sanitizeDoctorCacheMessages(messages);
    expect(result).toHaveLength(2);
    expect(result[0].id).toBe("good1");
    expect(result[1].id).toBe("good2");
  });
});

describe("extractOpenclawText", () => {
  test("extracts from payloads array", () => {
    const result = { payloads: [{ text: "hello" }, { text: "world" }] };
    expect(extractOpenclawText(result)).toBe("hello\nworld");
  });

  test("skips empty text in payloads", () => {
    const result = { payloads: [{ text: "hello" }, { text: "" }, { text: "world" }] };
    expect(extractOpenclawText(result)).toBe("hello\nworld");
  });

  test("falls back to text field", () => {
    const result = { text: "fallback text" };
    expect(extractOpenclawText(result)).toBe("fallback text");
  });

  test("falls back to content field", () => {
    const result = { content: "content fallback" };
    expect(extractOpenclawText(result)).toBe("content fallback");
  });

  test("returns empty string when no text found", () => {
    expect(extractOpenclawText({})).toBe("");
  });

  test("prefers payloads over text field", () => {
    const result = { payloads: [{ text: "from payload" }], text: "from text" };
    expect(extractOpenclawText(result)).toBe("from payload");
  });

  test("falls through empty payloads to text", () => {
    const result = { payloads: [], text: "from text" };
    expect(extractOpenclawText(result)).toBe("from text");
  });

  test("prefers text over content", () => {
    const result = { text: "from text", content: "from content" };
    expect(extractOpenclawText(result)).toBe("from text");
  });
});

describe("extractOpenclawSessionId", () => {
  test("extracts from meta.agentMeta.sessionId", () => {
    const result = { meta: { agentMeta: { sessionId: "sess-123" } } };
    expect(extractOpenclawSessionId(result)).toBe("sess-123");
  });

  test("returns undefined when meta is missing", () => {
    expect(extractOpenclawSessionId({})).toBeUndefined();
  });

  test("returns undefined when agentMeta is missing", () => {
    expect(extractOpenclawSessionId({ meta: {} })).toBeUndefined();
  });

  test("returns undefined for non-object meta", () => {
    expect(extractOpenclawSessionId({ meta: "string" })).toBeUndefined();
  });

  test("returns undefined for non-object agentMeta", () => {
    expect(extractOpenclawSessionId({ meta: { agentMeta: "string" } })).toBeUndefined();
  });

  test("returns undefined for non-string sessionId", () => {
    expect(extractOpenclawSessionId({ meta: { agentMeta: { sessionId: 42 } } })).toBeUndefined();
  });

  test("returns undefined for empty/whitespace sessionId", () => {
    expect(extractOpenclawSessionId({ meta: { agentMeta: { sessionId: "" } } })).toBeUndefined();
    expect(extractOpenclawSessionId({ meta: { agentMeta: { sessionId: "  " } } })).toBeUndefined();
  });

  test("trims sessionId", () => {
    expect(extractOpenclawSessionId({ meta: { agentMeta: { sessionId: "  sess-1  " } } })).toBe("sess-1");
  });
});

describe("isDoctorAutoSafeInvoke", () => {
  test("rejects non-doctor domain", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "doctor file read" } });
    expect(isDoctorAutoSafeInvoke(invoke, "install")).toBe(false);
  });

  test("approves safe clawpal doctor probe-openclaw", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "doctor probe-openclaw" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe clawpal doctor file read", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "doctor file read /path/to/file" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe clawpal doctor config-read", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "doctor config-read" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe clawpal doctor config-upsert", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "doctor config-upsert key value" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw --version", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "--version" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw doctor", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "doctor" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw health", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "health" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw config get", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "config get key" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw agents list", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "agents list" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("approves safe openclaw security audit", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "security audit" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("rejects unknown clawpal commands", () => {
    const invoke = makeInvoke({ command: "clawpal", args: { args: "deploy nuke" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(false);
  });

  test("rejects unknown openclaw commands", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "agents delete mybot" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(false);
  });

  test("rejects unknown command type", () => {
    const invoke = makeInvoke({ command: "bash", args: { args: "rm -rf /" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(false);
  });

  test("handles case insensitive args", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "DOCTOR" } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });

  test("handles extra whitespace in args", () => {
    const invoke = makeInvoke({ command: "openclaw", args: { args: "  config   get   key  " } });
    expect(isDoctorAutoSafeInvoke(invoke, "doctor")).toBe(true);
  });
});
