import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { InstanceCard } from "../InstanceCard";

describe("InstanceCard SSH connection profile", () => {
  test("shows only connection quality inline", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:hetzner",
          label: "hetzner",
          type: "ssh",
          healthy: false,
          agentCount: 1,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
          sshConnectionProfile: {
            status: {
              healthy: false,
              activeAgents: 1,
              sshDiagnostic: null,
            },
            connectLatencyMs: 120,
            gatewayLatencyMs: 90,
            configLatencyMs: 2420,
            versionLatencyMs: 250,
            totalLatencyMs: 2420,
            quality: "poor",
            qualityScore: 18,
            bottleneck: {
              stage: "config",
              latencyMs: 2420,
            },
          },
        }),
      }),
    );

    expect(html).toContain(">Poor<");
    expect(html).not.toContain("Poor · 2.42 s");
  });
});
