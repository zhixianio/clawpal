import { describe, expect, test } from "bun:test";

import type { RecipePlan } from "@/lib/types";
import {
  buildCookAuthProfileScope,
  buildCookContextWarnings,
  buildCookRouteSummary,
  filterCookAuthIssues,
  hasBlockingAuthIssues,
} from "../cook-plan-context";

const samplePlan: RecipePlan = {
  summary: {
    recipeId: "dedicated-channel-agent",
    recipeName: "Create dedicated Agent for Channel",
    executionKind: "job",
    actionCount: 4,
    skippedStepCount: 0,
  },
  usedCapabilities: ["agent.manage", "binding.manage", "config.write"],
  concreteClaims: [],
  executionSpecDigest: "digest-123",
  executionSpec: {
    apiVersion: "strategy.platform/v1",
    kind: "ExecutionSpec",
    metadata: { name: "dedicated-channel-agent" },
    source: {},
    target: {},
    execution: { kind: "job" as const },
    capabilities: { usedCapabilities: ["agent.manage"] },
    resources: { claims: [] },
    secrets: { bindings: [] },
    desiredState: {},
    actions: [
      {
        kind: "bind_channel",
        args: {
          channelType: "discord",
          peerId: "channel-1",
          agentId: "lobster",
        },
      },
      {
        kind: "config_patch",
        args: {
          patch: {
            channels: {
              discord: {
                guilds: {
                  "guild-1": {
                    channels: {
                      "channel-1": {
                        systemPrompt: "new persona",
                      },
                    },
                  },
                },
              },
            },
          },
        },
      },
    ],
    outputs: [],
  },
  warnings: [],
};

const authPlan: RecipePlan = {
  summary: {
    recipeId: "dedicated-agent",
    recipeName: "Dedicated Agent",
    executionKind: "job",
    actionCount: 4,
    skippedStepCount: 0,
  },
  usedCapabilities: ["model.manage", "secret.sync", "agent.manage"],
  concreteClaims: [{ kind: "modelProfile", id: "profile-openai" }],
  executionSpecDigest: "digest-auth-123",
  executionSpec: {
    apiVersion: "strategy.platform/v1",
    kind: "ExecutionSpec",
    metadata: { name: "dedicated-agent" },
    source: {},
    target: {},
    execution: { kind: "job" as const },
    capabilities: { usedCapabilities: ["model.manage"] },
    resources: { claims: [{ kind: "modelProfile", id: "profile-openai" }] },
    secrets: { bindings: [] },
    desiredState: {},
    actions: [
      {
        kind: "ensure_model_profile",
        args: {
          profileId: "profile-openai",
        },
      },
      {
        kind: "create_agent",
        args: {
          agentId: "ops-bot",
          modelProfileId: "profile-openai",
        },
      },
    ],
    outputs: [],
  },
  warnings: [],
};

describe("cook plan context helpers", () => {
  test("describes remote ssh execution routing", () => {
    expect(
      buildCookRouteSummary({
        instanceId: "ssh:prod-a",
        instanceLabel: "Prod A",
        isRemote: true,
        isDocker: false,
      }),
    ).toEqual({
      kind: "ssh",
      targetLabel: "Prod A",
    });
  });

  test("warns when a channel binding will be reassigned", () => {
    const warnings = buildCookContextWarnings(
      samplePlan,
      JSON.stringify({
        bindings: [
          {
            agentId: "main",
            match: {
              channel: "discord",
              peer: { kind: "channel", id: "channel-1" },
            },
          },
        ],
      }),
    );

    expect(warnings.some((warning) => warning.includes("will be rebound"))).toBe(true);
  });

  test("warns when channel config will overwrite an existing value", () => {
    const warnings = buildCookContextWarnings(
      samplePlan,
      JSON.stringify({
        channels: {
          discord: {
            guilds: {
              "guild-1": {
                channels: {
                  "channel-1": {
                    systemPrompt: "old persona",
                  },
                },
              },
            },
          },
        },
      }),
    );

    expect(warnings.some((warning) => warning.includes("systemPrompt"))).toBe(true);
  });

  test("treats auth errors as blocking", () => {
    expect(
      hasBlockingAuthIssues([
        {
          code: "AUTH_CREDENTIAL_UNRESOLVED",
          severity: "error",
          message: "missing auth",
          autoFixable: false,
        },
      ]),
    ).toBe(true);
  });

  test("builds auth scope from claimed and auto-prepared model profiles", () => {
    expect(buildCookAuthProfileScope(authPlan)).toEqual({
      requiredProfileIds: ["profile-openai"],
      autoPrepareProfileIds: ["profile-openai"],
    });
  });

  test("filters out unrelated auth blockers and suppresses auto-prepared profile issues", () => {
    const scope = buildCookAuthProfileScope(authPlan);
    const issues = filterCookAuthIssues(
      [
        {
          code: "AUTH_CREDENTIAL_UNRESOLVED",
          severity: "error",
          message: "Profile 'profile-openai' has no resolved credential for provider 'openai'",
          autoFixable: false,
        },
        {
          code: "AUTH_CREDENTIAL_UNRESOLVED",
          severity: "error",
          message: "Profile 'profile-anthropic' has no resolved credential for provider 'anthropic'",
          autoFixable: false,
        },
      ],
      scope,
    );

    expect(issues).toEqual([]);
  });
});
