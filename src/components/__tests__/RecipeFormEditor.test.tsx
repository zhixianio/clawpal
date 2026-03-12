import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { RecipeFormEditor } from "@/components/RecipeFormEditor";
import { parseRecipeSource, toRecipeEditorModel } from "@/lib/recipe-editor-model";

const SAMPLE_SOURCE = JSON.stringify({
  id: "channel-persona",
  name: "Channel Persona",
  description: "Apply a persona to one channel",
  version: "1.0.0",
  tags: ["discord", "persona"],
  difficulty: "easy",
  params: [
    {
      id: "guild_id",
      label: "Guild",
      type: "discord_guild",
      required: true,
    },
    {
      id: "channel_id",
      label: "Channel",
      type: "discord_channel",
      required: true,
      placeholder: "channel-123",
      pattern: "^[0-9]+$",
      minLength: 3,
      maxLength: 32,
      dependsOn: "guild_id",
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

describe("RecipeFormEditor", () => {
  test("renders add and remove controls for form authoring", async () => {
    await i18n.changeLanguage("en");
    const model = toRecipeEditorModel(parseRecipeSource(SAMPLE_SOURCE));

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(RecipeFormEditor, {
          model,
          readOnly: false,
          onChange: () => {},
        }),
      }),
    );

    expect(html).toContain("Add param");
    expect(html).toContain("Add step");
    expect(html).toContain("Add action");
    expect(html).toContain("Remove");
    expect(html).toContain("Required");
    expect(html).toContain("Placeholder");
    expect(html).toContain("Pattern");
    expect(html).toContain("Depends on");
    expect(html).toContain("Min length");
    expect(html).toContain("Max length");
    expect(html).toContain("channel-123");
    expect(html).toContain("guild_id");
  });
});
