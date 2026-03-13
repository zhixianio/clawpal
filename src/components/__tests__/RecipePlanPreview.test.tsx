import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { RecipePlanPreview } from "../RecipePlanPreview";

describe("RecipePlanPreview", () => {
  test("renders a non-technical review summary for confirm phase", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(RecipePlanPreview, {
          routeSummary: {
            kind: "ssh",
            targetLabel: "prod-a",
          },
          workspaceEntry: {
            slug: "channel-persona-pack",
            path: "/tmp/channel-persona-pack.recipe.json",
            recipeId: "channel-persona-pack",
            version: "1.0.0",
            sourceKind: "remoteUrl",
            bundledVersion: undefined,
            bundledState: undefined,
            trustLevel: "untrusted",
            riskLevel: "medium",
            approvalRequired: true,
          },
          authIssues: [
            {
              code: "AUTH_CREDENTIAL_UNRESOLVED",
              severity: "error",
              message: "missing auth",
              autoFixable: false,
            },
          ],
          contextWarnings: ["Channel discord/channel-1 will be rebound from main to lobster."],
          plan: {
            summary: {
              recipeId: "discord-channel-persona",
              recipeName: "Channel Persona",
              executionKind: "attachment",
              actionCount: 2,
              skippedStepCount: 1,
            },
            usedCapabilities: ["service.manage"],
            concreteClaims: [
              { kind: "channel", id: "channel-1" },
              { kind: "file", path: "~/.openclaw/config.json" },
            ],
            executionSpecDigest: "digest-123",
            executionSpec: {
              apiVersion: "strategy.platform/v1",
              kind: "ExecutionSpec",
              metadata: {},
              source: {},
              target: {},
              execution: {
                kind: "attachment",
              },
              capabilities: {
                usedCapabilities: ["service.manage"],
              },
              resources: {
                claims: [
                  { kind: "channel", id: "channel-1" },
                  { kind: "file", path: "~/.openclaw/config.json" },
                ],
              },
              secrets: {
                bindings: [],
              },
              desiredState: {},
              actions: [
                { kind: "config_patch", name: "Apply channel persona preset", args: {} },
                { kind: "config_patch", name: "Reload channel prompt", args: {} },
              ],
              outputs: [],
            },
            warnings: [],
          },
        }),
      }),
    );

    expect(html).toContain("What this recipe will do");
    expect(html).toContain("Apply channel persona preset");
    expect(html).toContain("Where changes will be applied");
    expect(html).toContain("SSH");
    expect(html).toContain("prod-a");
    expect(html).toContain("What will be updated");
    expect(html).toContain("What must be ready before this runs");
    expect(html).toContain("Why this cannot continue yet");
    expect(html).toContain("Approve the current saved version before you continue.");
    expect(html).toContain("Needs attention before you continue");
    expect(html).toContain("Advanced details");
    expect(html).toContain("digest-123");
    expect(html).not.toContain("Capabilities");
    expect(html).not.toContain("Resource Claims");
    expect(html).not.toContain("Auth Preconditions");
  });
});
