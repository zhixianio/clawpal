import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { DoctorChatToolbar } from "../DoctorChatToolbar";

describe("DoctorChatToolbar", () => {
  test("keeps only subtle clear icon and full-auto controls in the doctor chat header", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorChatToolbar, {
          fullAuto: false,
          onFullAutoChange: () => {},
          onClear: () => {},
        }),
      }),
    );

    expect(html).toContain('aria-label="Clear"');
    expect(html).toContain('data-size="icon-sm"');
    expect(html).toContain("Full Auto");
    expect(html).not.toContain("Doctor Claw Assistant");
    expect(html).not.toContain("Session override");
    expect(html).not.toContain("gpt-5.3-codex");
    expect(html).not.toContain(">Clear<");
  });

  test("renders a disabled clear button state when requested", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(DoctorChatToolbar, {
          fullAuto: true,
          clearDisabled: true,
          onFullAutoChange: () => {},
          onClear: () => {},
        }),
      }),
    );

    expect(html).toContain('aria-label="Clear"');
    expect(html).toContain("disabled");
    expect(html).toContain("checked");
  });
});
