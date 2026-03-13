import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { InstanceContext } from "@/lib/instance-context";
import { ParamForm } from "../ParamForm";

describe("ParamForm", () => {
  test("keeps submit enabled before invalid fields are touched", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceContext.Provider, {
          value: {
            instanceId: "local",
            instanceViewToken: "local",
            instanceToken: 0,
            persistenceScope: "local",
            persistenceResolved: true,
            isRemote: false,
            isDocker: false,
            isConnected: true,
            channelNodes: null,
            discordGuildChannels: [],
            channelsLoading: false,
            discordChannelsLoading: false,
            refreshChannelNodesCache: async () => [],
            refreshDiscordChannelsCache: async () => [],
          },
          children: React.createElement(ParamForm, {
            recipe: {
              id: "dedicated-agent",
              name: "Dedicated Agent",
              description: "Create an independent agent",
              version: "1.0.0",
              tags: ["agent"],
              difficulty: "easy",
              params: [
                {
                  id: "agent_id",
                  label: "Agent ID",
                  type: "string",
                  required: true,
                },
              ],
              steps: [],
            },
            values: { agent_id: "" },
            onChange: () => {},
            onSubmit: () => {},
            submitLabel: "Next",
          }),
        }),
      }),
    );

    expect(html).toContain(">Next</button>");
    expect(html).not.toMatch(/<button[^>]*\sdisabled(?:=""|>|\s)/);
  });

  test("renders preset options as a select list", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceContext.Provider, {
          value: {
            instanceId: "local",
            instanceViewToken: "local",
            instanceToken: 0,
            persistenceScope: "local",
            persistenceResolved: true,
            isRemote: false,
            isDocker: false,
            isConnected: true,
            channelNodes: null,
            discordGuildChannels: [],
            channelsLoading: false,
            discordChannelsLoading: false,
            refreshChannelNodesCache: async () => [],
            refreshDiscordChannelsCache: async () => [],
          },
          children: React.createElement(ParamForm, {
            recipe: {
              id: "agent-persona-pack",
              name: "Agent Persona Pack",
              description: "Import persona presets into an agent",
              version: "1.0.0",
              tags: ["agent", "persona"],
              difficulty: "easy",
              params: [
                {
                  id: "persona_preset",
                  label: "Persona preset",
                  type: "string",
                  required: true,
                  options: [
                    { value: "friendly", label: "Friendly" },
                    { value: "ops", label: "Ops" },
                  ],
                },
              ],
              steps: [],
            },
            values: { persona_preset: "friendly" },
            onChange: () => {},
            onSubmit: () => {},
          }),
        }),
      }),
    );

    expect(html).toContain("Persona preset");
    expect(html).toContain('role="combobox"');
    expect(html).not.toContain("<textarea");
  });
});
