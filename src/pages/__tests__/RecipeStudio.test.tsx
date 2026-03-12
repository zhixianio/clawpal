import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { RecipeStudio } from "../RecipeStudio";

describe("RecipeStudio", () => {
  test("renders editable source mode for workspace drafts", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(RecipeStudio, {
          recipeId: "channel-persona",
          recipeName: "Channel Persona",
          initialSource: JSON.stringify({
            id: "channel-persona",
            name: "Channel Persona",
            description: "Apply a persona to one channel",
            version: "1.0.0",
            tags: ["discord"],
            difficulty: "easy",
            params: [],
            steps: [],
            bundle: {
              apiVersion: "strategy.platform/v1",
              kind: "StrategyBundle",
              metadata: {},
              compatibility: {},
              inputs: [],
              capabilities: { allowed: [] },
              resources: { supportedKinds: [] },
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
              capabilities: { usedCapabilities: [] },
              resources: { claims: [] },
              secrets: { bindings: [] },
              desiredState: {},
              actions: [],
              outputs: [],
            },
          }, null, 2),
          origin: "workspace",
          onCookDraft: () => {},
          onBack: () => {},
        }),
      }),
    );

    expect(html).toContain("Recipe Studio");
    expect(html).toContain("Channel Persona");
    expect(html).toContain("Editable draft");
    expect(html).toContain("New");
    expect(html).toContain("Save");
    expect(html).toContain("Save as");
    expect(html).toContain("Cook draft");
    expect(html).toContain("Preview plan");
    expect(html).toContain("Form");
    expect(html).toContain("textarea");
    expect(html).toContain("ExecutionSpec");
  });

  test("shows source/form diff hints when source uses a wrapped recipe document", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(RecipeStudio, {
          recipeId: "channel-persona",
          recipeName: "Channel Persona",
          initialSource: JSON.stringify({
            recipes: [{
              id: "channel-persona",
              name: "Channel Persona",
              description: "Apply a persona to one channel",
              version: "1.0.0",
              tags: ["discord"],
              difficulty: "easy",
              params: [],
              steps: [],
              bundle: {
                apiVersion: "strategy.platform/v1",
                kind: "StrategyBundle",
                metadata: {},
                compatibility: {},
                inputs: [],
                capabilities: { allowed: [] },
                resources: { supportedKinds: [] },
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
                capabilities: { usedCapabilities: [] },
                resources: { claims: [] },
                secrets: { bindings: [] },
                desiredState: {},
                actions: [],
                outputs: [],
              },
            }],
          }, null, 2),
          origin: "workspace",
          onCookDraft: () => {},
          onBack: () => {},
        }),
      }),
    );

    expect(html).toContain("Source/form diff");
    expect(html).toContain("document shape");
  });
});
