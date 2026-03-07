import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { DoctorDisclosureSection } from "../DoctorDisclosureSection";

describe("DoctorDisclosureSection", () => {
  test("renders collapsed by default using the settings disclosure card shell", () => {
    const html = renderToStaticMarkup(
      React.createElement(DoctorDisclosureSection, {
        title: "Sessions",
        children: React.createElement("div", null, "Content"),
      }),
    );

    expect(html).toContain("<details");
    expect(html).not.toContain("<details open");
    expect(html).toContain(">Sessions<");
    expect(html).toContain('data-slot="card"');
    expect(html).toContain("bg-muted/20");
  });
});
