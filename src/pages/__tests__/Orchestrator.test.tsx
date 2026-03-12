import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { InstanceContext } from "@/lib/instance-context";
import { Orchestrator } from "../Orchestrator";

describe("Orchestrator runtime timeline", () => {
  test("shows artifacts and resource claims in orchestrator", async () => {
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
          children: React.createElement(Orchestrator, {
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
                artifacts: [
                  {
                    id: "artifact_01",
                    kind: "configDiff",
                    label: "Rendered patch",
                    path: "/tmp/rendered-patch.json",
                  },
                ],
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
            initialEvents: [],
          }),
        }),
      }),
    );

    expect(html).toContain("Resource claims");
    expect(html).toContain("Rendered patch");
    expect(html).toContain("openclaw.config");
    expect(html).toContain("digest-123");
  });
});
