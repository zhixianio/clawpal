import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { InstanceContext } from "@/lib/instance-context";
import { Recipes } from "../Recipes";

describe("Recipes runtime summary", () => {
  test("shows recipe instance status and recent run summary", async () => {
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
            discordGuildChannels: null,
            channelsLoading: false,
            discordChannelsLoading: false,
            refreshChannelNodesCache: async () => [],
            refreshDiscordChannelsCache: async () => [],
          },
          children: React.createElement(Recipes, {
            onCook: () => {},
            onOpenStudio: () => {},
            initialRecipes: [
              {
                id: "discord-channel-persona",
                name: "Channel Persona",
                description: "Apply a persona to one channel",
                version: "1.0.0",
                tags: ["discord", "persona"],
                difficulty: "normal",
                params: [],
                steps: [],
              },
            ],
            initialInstances: [
              {
                id: "local",
                recipeId: "discord-channel-persona",
                executionKind: "attachment",
                runner: "local",
                status: "succeeded",
                lastRunId: "run_01",
                updatedAt: "2026-03-11T10:00:03Z",
              },
            ],
            initialRuns: [
              {
                id: "run_01",
                instanceId: "local",
                recipeId: "discord-channel-persona",
                executionKind: "attachment",
                runner: "local",
                status: "succeeded",
                summary: "Applied persona patch",
                startedAt: "2026-03-11T10:00:00Z",
                finishedAt: "2026-03-11T10:00:03Z",
                artifacts: [],
              resourceClaims: [
                {
                  kind: "path",
                  id: "openclaw.config",
                  path: "~/.openclaw/config.json",
                },
              ],
              warnings: [],
              sourceOrigin: "draft",
              sourceDigest: "digest-123",
              workspacePath: "/tmp/channel-persona.recipe.json",
            },
          ],
        }),
        }),
      }),
    );

    expect(html).toContain("Recent run");
    expect(html).toContain("Applied persona patch");
    expect(html).toContain("succeeded");
    expect(html).toContain("View source");
    expect(html).toContain("Fork to workspace");
    expect(html).toContain("digest-123");
  });
});
