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
          initialSource: '{\n  "kind": "ExecutionSpec"\n}',
          origin: "workspace",
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
    expect(html).toContain("textarea");
    expect(html).toContain("ExecutionSpec");
  });
});
