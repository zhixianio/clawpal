import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { ParamForm } from "@/components/ParamForm";

const SAMPLE_RECIPE = {
  id: "channel-persona",
  name: "Channel Persona",
  description: "Apply a persona to one channel",
  version: "1.0.0",
  tags: ["discord"],
  difficulty: "easy" as const,
  params: [
    {
      id: "guild_id",
      label: "Guild",
      type: "discord_guild" as const,
      required: true,
    },
    {
      id: "channel_id",
      label: "Channel",
      type: "discord_channel" as const,
      required: true,
      dependsOn: "guild_id",
    },
  ],
  steps: [],
};

describe("ParamForm", () => {
  test("shows dependent params when the upstream string param has a value", async () => {
    await i18n.changeLanguage("en");

    const emptyHtml = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(ParamForm, {
          recipe: SAMPLE_RECIPE,
          values: {
            guild_id: "",
            channel_id: "",
          },
          onChange: () => {},
          onSubmit: () => {},
        }),
      }),
    );

    const selectedHtml = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(ParamForm, {
          recipe: SAMPLE_RECIPE,
          values: {
            guild_id: "guild-123",
            channel_id: "",
          },
          onChange: () => {},
          onSubmit: () => {},
        }),
      }),
    );

    expect(emptyHtml).not.toContain("Channel");
    expect(selectedHtml).toContain("Channel");
  });
});
