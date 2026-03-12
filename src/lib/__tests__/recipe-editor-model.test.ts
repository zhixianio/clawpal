import { describe, expect, test } from "bun:test";

import {
  fromRecipeEditorModel,
  parseRecipeSource,
  toRecipeEditorModel,
} from "@/lib/recipe-editor-model";

const SAMPLE_SOURCE = JSON.stringify({
  id: "channel-persona",
  name: "Channel Persona",
  description: "Apply a persona to one channel",
  version: "1.0.0",
  tags: ["discord", "persona"],
  difficulty: "easy",
  params: [
    {
      id: "channel_id",
      label: "Channel",
      type: "discord_channel",
      required: true,
    },
  ],
  steps: [
    {
      action: "config_patch",
      label: "Set persona",
      args: {
        patchTemplate: "{\"channels\":{}}",
      },
    },
  ],
  bundle: {
    apiVersion: "strategy.platform/v1",
    kind: "StrategyBundle",
    metadata: {},
    compatibility: {},
    inputs: [],
    capabilities: { allowed: ["config.write"] },
    resources: { supportedKinds: ["path"] },
    execution: { supportedKinds: ["attachment"] },
    runner: {},
    outputs: [],
  },
  executionSpecTemplate: {
    apiVersion: "strategy.platform/v1",
    kind: "ExecutionSpec",
    metadata: {},
    source: {},
    target: {},
    execution: { kind: "attachment" },
    capabilities: { usedCapabilities: ["config.write"] },
    resources: { claims: [] },
    secrets: { bindings: [] },
    desiredState: {},
    actions: [
      {
        kind: "config_patch",
        name: "Set persona",
        args: {
          patch: {
            channels: {},
          },
        },
      },
    ],
    outputs: [],
  },
}, null, 2);

describe("recipe editor model", () => {
  test("round-trips metadata params steps and execution template", () => {
    const doc = parseRecipeSource(SAMPLE_SOURCE);
    const form = toRecipeEditorModel(doc);
    const nextDoc = fromRecipeEditorModel(form);

    expect(nextDoc.executionSpecTemplate?.kind).toBe("ExecutionSpec");
    expect(nextDoc.id).toBe("channel-persona");
    expect(nextDoc.params[0].id).toBe("channel_id");
    expect(nextDoc.steps[0].action).toBe("config_patch");
  });
});
