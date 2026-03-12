import { describe, expect, test } from "bun:test";

import {
  appendRecipeEditorActionRow,
  appendRecipeEditorParam,
  appendRecipeEditorStep,
  fromRecipeEditorModel,
  parseRecipeSource,
  removeRecipeEditorActionRow,
  removeRecipeEditorParam,
  removeRecipeEditorStep,
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

  test("appends and removes params steps and actions with sensible defaults", () => {
    const doc = parseRecipeSource(SAMPLE_SOURCE);
    const form = toRecipeEditorModel(doc);

    const appendedParam = appendRecipeEditorParam({ ...form, params: [] });
    expect(appendedParam.params).toHaveLength(1);
    expect(appendedParam.params[0]).toMatchObject({
      id: "param_1",
      label: "Param 1",
      type: "string",
      required: false,
    });
    expect(removeRecipeEditorParam(appendedParam, 0).params).toHaveLength(0);

    const appendedStep = appendRecipeEditorStep({ ...form, steps: [] });
    expect(appendedStep.steps).toHaveLength(1);
    expect(appendedStep.steps[0]).toMatchObject({
      label: "Step 1",
      action: "config_patch",
      args: {},
    });
    expect(removeRecipeEditorStep(appendedStep, 0).steps).toHaveLength(0);

    const appendedAction = appendRecipeEditorActionRow({ ...form, actionRows: [] });
    expect(appendedAction.actionRows).toHaveLength(1);
    expect(appendedAction.actionRows[0]).toMatchObject({
      kind: "config_patch",
      name: "Action 1",
      argsText: "{}",
    });
    expect(removeRecipeEditorActionRow(appendedAction, 0).actionRows).toHaveLength(0);
  });
});
