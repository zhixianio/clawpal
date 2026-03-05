import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  applyCheckedToggle,
  DiagnosisCard,
  DiagnosisCardView,
  formatConfidence,
  formatJson,
  formatMarkdown,
  handleDiagnosisExport,
  safeClipboardWrite,
  toggleExportMenu,
  toggleCheckedState,
} from "../DiagnosisCard";
import type { DiagnosisReportItem } from "@/lib/types";

describe("DiagnosisCard helpers", () => {
  test("formatConfidence clamps and converts to percent", () => {
    expect(formatConfidence(undefined)).toBeNull();
    expect(formatConfidence(Number.NaN)).toBeNull();
    expect(formatConfidence(-0.2)).toBe("0%");
    expect(formatConfidence(0.726)).toBe("73%");
    expect(formatConfidence(1.5)).toBe("100%");
  });

  test("formatMarkdown includes structured diagnosis fields", () => {
    const items: DiagnosisReportItem[] = [
      {
        problem: "Gateway auth mismatch",
        severity: "error",
        fix_options: ["legacy fallback option"],
        root_cause_hypothesis: "Proxy dropped authorization header",
        fix_steps: ["Verify proxy header forwarding", "Restart gateway"],
        confidence: 0.84,
        citations: [
          { url: "https://docs.openclaw.ai/cli/gateway", section: "Gateway" },
          { url: "https://docs.openclaw.ai/automation/troubleshooting" },
        ],
        version_awareness: "Prefer target-host local docs for installed OpenClaw version 2026.3.1",
      },
    ];

    const md = formatMarkdown(items);
    expect(md).toContain("Root cause hypothesis");
    expect(md).toContain("**Confidence:** 84%");
    expect(md).toContain("Fix steps");
    expect(md).toContain("Verify proxy header forwarding");
    expect(md).toContain("https://docs.openclaw.ai/cli/gateway");
    expect(md).toContain("Version awareness");
  });

  test("formatJson preserves structured fields", () => {
    const items: DiagnosisReportItem[] = [
      {
        problem: "Provider auth failure",
        severity: "warn",
        fix_options: [],
        root_cause_hypothesis: "auth_ref points to missing key",
        fix_steps: ["Run openclaw auth list"],
        confidence: 0.61,
        citations: [{ url: "https://docs.openclaw.ai/auth-credential-semantics" }],
      },
    ];
    const json = formatJson(items);
    expect(json).toContain("\"root_cause_hypothesis\"");
    expect(json).toContain("\"fix_steps\"");
    expect(json).toContain("\"citations\"");
  });

  test("toggleCheckedState flips checkbox state by index", () => {
    const first = toggleCheckedState({}, 2);
    expect(first[2]).toBe(true);
    const second = toggleCheckedState(first, 2);
    expect(second[2]).toBe(false);
  });

  test("handleDiagnosisExport writes markdown and updates copied/export state", async () => {
    const items: DiagnosisReportItem[] = [
      {
        problem: "Gateway auth mismatch",
        severity: "error",
        fix_options: ["legacy fallback option"],
      },
    ];
    const writes: string[] = [];
    const copiedStates: boolean[] = [];
    const exportStates: boolean[] = [];
    const timerDelays: number[] = [];

    await handleDiagnosisExport(
      items,
      "markdown",
      async (text) => {
        writes.push(text);
      },
      (value) => copiedStates.push(value),
      (value) => exportStates.push(value),
      (callback, delayMs) => {
        timerDelays.push(delayMs);
        callback();
        return 0;
      },
    );

    expect(exportStates).toEqual([false]);
    expect(copiedStates).toEqual([true, false]);
    expect(timerDelays).toEqual([1500]);
    expect(writes[0]).toContain("## 1. [ERROR] Gateway auth mismatch");
  });

  test("safeClipboardWrite resolves without clipboard API", async () => {
    await expect(safeClipboardWrite("hello")).resolves.toBeUndefined();
  });

  test("toggleExportMenu flips open state", () => {
    let next: boolean | undefined;
    toggleExportMenu(false, (value) => {
      next = value;
    });
    expect(next).toBe(true);
  });

  test("applyCheckedToggle updates checked state through setter callback", () => {
    let computed: Record<number, boolean> | undefined;
    applyCheckedToggle(1, (updater) => {
      const updateFn = updater as (prev: Record<number, boolean>) => Record<number, boolean>;
      computed = updateFn({ 1: false });
    });
    expect(computed?.[1]).toBe(true);
  });

  test("DiagnosisCardView renders structured fields and action button", () => {
    const items: DiagnosisReportItem[] = [
      {
        problem: "Tool policy denied elevated command",
        severity: "error",
        fix_options: [],
        root_cause_hypothesis: "tools.elevated policy blocked command scope",
        fix_steps: ["Review tools policy", "Approve least-privilege rule"],
        confidence: 0.91,
        citations: [{ url: "https://docs.openclaw.ai/cli/sandbox", section: "Sandbox CLI" }],
        version_awareness: "Prefer local docs matching installed OpenClaw version",
        action: { tool: "clawpal", args: "doctor config-read tools" },
      },
    ];
    const html = renderToStaticMarkup(
      React.createElement(DiagnosisCardView, {
        items,
        t: ((key: string, options?: { defaultValue?: string }) =>
          options?.defaultValue ?? key) as never,
      })
    );
    expect(html).toContain("Tool policy denied elevated command");
    expect(html).toContain("91%");
    expect(html).toContain("tools.elevated policy blocked command scope");
    expect(html).toContain("https://docs.openclaw.ai/cli/sandbox");
    expect(html).toContain("Auto-fix");
  });

  test("DiagnosisCard wrapper renders with i18n hook", () => {
    const html = renderToStaticMarkup(
      React.createElement(DiagnosisCard, {
        items: [
          {
            problem: "Provider auth failure",
            severity: "warn",
            fix_options: [],
          },
        ],
      }),
    );
    expect(html.includes("Diagnosis Report") || html.includes("doctor.diagnosisReport")).toBe(true);
  });
});
