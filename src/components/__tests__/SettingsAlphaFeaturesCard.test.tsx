import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { SettingsAlphaFeaturesCard } from "../SettingsAlphaFeaturesCard";

describe("SettingsAlphaFeaturesCard", () => {
  test("shows ssh transfer speed, logs, and context toggles", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(SettingsAlphaFeaturesCard, {
          showSshTransferSpeedUi: false,
          showClawpalLogsUi: true,
          showGatewayLogsUi: false,
          showOpenclawContextUi: true,
          onSshTransferSpeedUiToggle: () => {},
          onClawpalLogsUiToggle: () => {},
          onGatewayLogsUiToggle: () => {},
          onOpenclawContextUiToggle: () => {},
        }),
      }),
    );

    expect(html).toContain("SSH transfer speed");
    expect(html).toContain("ClawPal Logs");
    expect(html).toContain("OpenClaw Gateway Logs");
    expect(html).toContain("OpenClaw Context");
    expect(html).not.toContain("Enable Doctor Claw (Alpha)");
    expect(html).not.toContain("Enable Rescue Bot (Alpha)");
  });
});
