import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { ClawpalLogs } from "../ClawpalLogs";

describe("ClawpalLogs", () => {
  test("renders the local logs workspace with log controls", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(ClawpalLogs),
      }),
    );

    expect(html).toContain("ClawPal Logs");
    expect(html).toContain("App Log");
    expect(html).toContain("Error Log");
    expect(html).toContain("Refresh");
    expect(html).toContain("Export");
  });
});
