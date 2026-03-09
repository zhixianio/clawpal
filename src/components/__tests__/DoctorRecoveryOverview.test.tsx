import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import type { RescuePrimaryDiagnosisResult } from "@/lib/types";
import { DoctorRecoveryOverview } from "../DoctorRecoveryOverview";

describe("DoctorRecoveryOverview", () => {
  test("renders a concise summary first and keeps the section checklist visible by default", async () => {
    await i18n.changeLanguage("en");
    const diagnosis: RescuePrimaryDiagnosisResult = {
      status: "broken",
      checkedAt: "2026-03-07T00:00:00Z",
      targetProfile: "primary",
      rescueProfile: "rescue",
      rescueConfigured: true,
      rescuePort: 19789,
      summary: {
        status: "broken",
        headline: "Gateway needs attention first",
        recommendedAction: "Apply 1 fix and re-run recovery",
        fixableIssueCount: 1,
        selectedFixIssueIds: ["field.agents"],
        rootCauseHypotheses: [
          {
            title: "Agent defaults are missing",
            reason: "The primary profile has no agents.defaults.model binding.",
            score: 0.91,
          },
        ],
        fixSteps: [
          "Set agents.defaults.model to a valid provider/model pair.",
          "Re-run the primary check after saving the config.",
        ],
        confidence: 0.91,
        citations: [
          {
            url: "https://docs.openclaw.ai/agents",
            section: "defaults",
          },
        ],
        versionAwareness: "Guidance matches OpenClaw 2026.3.x.",
      },
      sections: [
        {
          key: "agents",
          title: "Agents",
          status: "degraded",
          summary: "Agents has 1 recommended change",
          docsUrl: "https://docs.openclaw.ai/agents",
          rootCauseHypotheses: [
            {
              title: "Agent defaults are missing",
              reason: "The primary profile has no agents.defaults.model binding.",
              score: 0.91,
            },
          ],
          fixSteps: [
            "Set agents.defaults.model to a valid provider/model pair.",
            "Re-run the primary check after saving the config.",
          ],
          confidence: 0.91,
          citations: [
            {
              url: "https://docs.openclaw.ai/agents",
              section: "defaults",
            },
          ],
          versionAwareness: "Guidance matches OpenClaw 2026.3.x.",
          items: [
            {
              id: "field.agents",
              label: "Missing agent defaults",
              status: "warn",
              detail: "Initialize agents.defaults.model",
              autoFixable: true,
              issueId: "field.agents",
            },
          ],
        },
        {
          key: "gateway",
          title: "Gateway",
          status: "healthy",
          summary: "Gateway checks look healthy",
          docsUrl: "https://docs.openclaw.ai/gateway/security/index",
          items: [
            {
              id: "field.gateway.port",
              label: "Gateway port",
              status: "ok",
              detail: "Configured primary gateway port: 18789",
              autoFixable: false,
              issueId: null,
            },
          ],
        },
      ],
      checks: [],
      issues: [],
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorRecoveryOverview, {
          diagnosis,
          checkLoading: false,
          repairing: false,
          progressLine: null,
          repairResult: null,
          repairError: null,
          onRepairAll: () => {},
          onRepairIssue: () => {},
        }),
      }),
    );

    expect(html).toContain("Gateway needs attention first");
    expect(html).toContain("Apply 1 fix and re-run recovery");
    expect(html).toContain("Fix 1 issue");
    expect(html).toContain("Agents");
    expect(html).toContain("Agent defaults are missing");
    expect(html).toContain("The primary profile has no agents.defaults.model binding.");
    expect(html).toContain("Set agents.defaults.model to a valid provider/model pair.");
    expect(html).toContain("Guidance matches OpenClaw 2026.3.x.");
    expect(html).toContain("Missing agent defaults");
    expect(html).toContain("Gateway port");
    expect(html.match(/Fix 1 issue/g)?.length ?? 0).toBe(1);
    expect(html).not.toContain("Open Gateway docs");
    expect(html).toContain("<details open");
  });

  test("uses optimize wording for degraded recommendations", async () => {
    await i18n.changeLanguage("en");
    const diagnosis: RescuePrimaryDiagnosisResult = {
      status: "degraded",
      checkedAt: "2026-03-07T00:00:00Z",
      targetProfile: "primary",
      rescueProfile: "rescue",
      rescueConfigured: true,
      rescuePort: 19789,
      summary: {
        status: "degraded",
        headline: "Gateway has recommended improvements",
        recommendedAction: "Apply 2 optimizations to stabilize the target",
        fixableIssueCount: 2,
        selectedFixIssueIds: ["tools.allowlist.review", "channel.policy.review"],
        rootCauseHypotheses: [],
        fixSteps: [],
        confidence: undefined,
        citations: [],
        versionAwareness: undefined,
      },
      sections: [
        {
          key: "gateway",
          title: "Gateway",
          status: "degraded",
          summary: "Gateway has 2 recommended changes",
          docsUrl: "https://docs.openclaw.ai/gateway/security/index",
          rootCauseHypotheses: [],
          fixSteps: [],
          confidence: undefined,
          citations: [],
          versionAwareness: undefined,
          items: [
            {
              id: "tools.allowlist.review",
              label: "Review helper permissions",
              status: "warn",
              detail: "Allowlist blocks rescue helper access",
              autoFixable: true,
              issueId: "tools.allowlist.review",
            },
          ],
        },
      ],
      checks: [],
      issues: [],
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorRecoveryOverview, {
          diagnosis,
          checkLoading: false,
          repairing: false,
          progressLine: null,
          repairResult: null,
          repairError: null,
          onRepairAll: () => {},
          onRepairIssue: () => {},
        }),
      }),
    );

    expect(html).toContain("Optimize 2 issues");
    expect(html).toContain("Optimize");
    expect(html).not.toContain("Fix 2 issues");
  });

  test("shows the broken badge once when summary, section, and item describe the same blocker", async () => {
    await i18n.changeLanguage("en");
    const diagnosis: RescuePrimaryDiagnosisResult = {
      status: "broken",
      checkedAt: "2026-03-07T00:00:00Z",
      targetProfile: "primary",
      rescueProfile: "rescue",
      rescueConfigured: true,
      rescuePort: 19789,
      summary: {
        status: "broken",
        headline: "Gateway needs attention first",
        recommendedAction: "Repair the blocking config error",
        fixableIssueCount: 1,
        selectedFixIssueIds: ["primary.config.unreadable"],
        rootCauseHypotheses: [],
        fixSteps: [],
        confidence: undefined,
        citations: [],
        versionAwareness: undefined,
      },
      sections: [
        {
          key: "gateway",
          title: "Gateway",
          status: "broken",
          summary: "Gateway has 1 blocking finding",
          docsUrl: "https://docs.openclaw.ai/gateway",
          rootCauseHypotheses: [],
          fixSteps: [],
          confidence: undefined,
          citations: [],
          versionAwareness: undefined,
          items: [
            {
              id: "primary.config.unreadable",
              label: "Primary configuration could not be read",
              status: "error",
              detail: "Repair the syntax error in ~/.openclaw/openclaw.json",
              autoFixable: true,
              issueId: "primary.config.unreadable",
            },
          ],
        },
      ],
      checks: [],
      issues: [],
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorRecoveryOverview, {
          diagnosis,
          checkLoading: false,
          repairing: false,
          progressLine: null,
          repairResult: null,
          repairError: null,
          onRepairAll: () => {},
          onRepairIssue: () => {},
        }),
      }),
    );

    expect(html.match(/>Broken</g)?.length ?? 0).toBe(1);
    expect(html).toContain("Primary configuration could not be read");
  });

  test("localizes the diagnosis report when display language is Chinese", async () => {
    await i18n.changeLanguage("zh");
    const diagnosis: RescuePrimaryDiagnosisResult = {
      status: "broken",
      checkedAt: "2026-03-07T00:00:00Z",
      targetProfile: "primary",
      rescueProfile: "rescue",
      rescueConfigured: true,
      rescuePort: 19789,
      summary: {
        status: "broken",
        headline: "Gateway needs attention first",
        recommendedAction: "Apply 1 fix and re-run recovery",
        fixableIssueCount: 1,
        selectedFixIssueIds: ["primary.config.unreadable"],
        rootCauseHypotheses: [],
        fixSteps: [],
        confidence: undefined,
        citations: [],
        versionAwareness: undefined,
      },
      sections: [
        {
          key: "gateway",
          title: "Gateway",
          status: "broken",
          summary: "Gateway has 1 blocking finding",
          docsUrl: "https://docs.openclaw.ai/gateway",
          rootCauseHypotheses: [],
          fixSteps: [],
          confidence: undefined,
          citations: [],
          versionAwareness: undefined,
          items: [
            {
              id: "primary.config.unreadable",
              label: "Primary configuration could not be read",
              status: "error",
              detail: "Repair openclaw.json parsing errors and re-run the primary recovery check",
              autoFixable: true,
              issueId: "primary.config.unreadable",
            },
            {
              id: "gateway.config.port",
              label: "Gateway port",
              status: "ok",
              detail: "Configured primary gateway port: 18789",
              autoFixable: false,
              issueId: null,
            },
          ],
        },
      ],
      checks: [],
      issues: [],
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorRecoveryOverview, {
          diagnosis,
          checkLoading: false,
          repairing: false,
          progressLine: null,
          repairResult: null,
          repairError: null,
          onRepairAll: () => {},
          onRepairIssue: () => {},
        }),
      }),
    );

    expect(html).toContain("网关需要优先处理");
    expect(html).toContain("应用 1 个修复后重新检查");
    expect(html).toContain("修复 1 个问题");
    expect(html).toContain("网关");
    expect(html).toContain("网关有 1 个阻塞性问题");
    expect(html).toContain("无法读取 Primary 配置");
    expect(html).toContain("请修复 openclaw.json 解析错误后重新运行 Primary 恢复检查");
    expect(html).toContain("网关端口");
    expect(html).toContain("Primary 网关端口已配置为 18789");
  });

  test("hides the summary card entirely when diagnosis is already healthy", async () => {
    await i18n.changeLanguage("en");
    const diagnosis: RescuePrimaryDiagnosisResult = {
      status: "healthy",
      checkedAt: "2026-03-07T00:00:00Z",
      targetProfile: "primary",
      rescueProfile: "rescue",
      rescueConfigured: true,
      rescuePort: 19789,
      summary: {
        status: "healthy",
        headline: "Primary recovery checks look healthy",
        recommendedAction: "Keep monitoring Gateway and re-run checks after changes",
        fixableIssueCount: 0,
        selectedFixIssueIds: [],
        rootCauseHypotheses: [],
        fixSteps: [],
        confidence: undefined,
        citations: [],
        versionAwareness: undefined,
      },
      sections: [
        {
          key: "gateway",
          title: "Gateway",
          status: "healthy",
          summary: "Gateway checks look healthy",
          docsUrl: "https://docs.openclaw.ai/gateway",
          rootCauseHypotheses: [],
          fixSteps: [],
          confidence: undefined,
          citations: [],
          versionAwareness: undefined,
          items: [
            {
              id: "gateway.port",
              label: "Gateway port",
              status: "ok",
              detail: "Configured primary gateway port: 18789",
              autoFixable: false,
              issueId: null,
            },
          ],
        },
      ],
      checks: [],
      issues: [],
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorRecoveryOverview, {
          diagnosis,
          checkLoading: false,
          repairing: false,
          progressLine: null,
          repairResult: null,
          repairError: null,
          onRepairAll: () => {},
          onRepairIssue: () => {},
        }),
      }),
    );

    expect(html).not.toContain("Primary recovery checks look healthy");
    expect(html).not.toContain("Keep monitoring Gateway and re-run checks after changes");
    expect(html).toContain("Gateway");
    expect(html).toContain("Gateway checks look healthy");
  });
});
