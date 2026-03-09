import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { RescueAsciiHeader } from "../RescueAsciiHeader";

describe("RescueAsciiHeader", () => {
  test("renders the doctor image header and progress slots for the current state", () => {
    const activeHtml = renderToStaticMarkup(
      React.createElement(RescueAsciiHeader, {
        state: "active",
        title: "Helper is enabled",
        progress: 0.5,
        animateProgress: true,
      }),
    );
    const pausedHtml = renderToStaticMarkup(
      React.createElement(RescueAsciiHeader, {
        state: "configured_inactive",
        title: "Helper is paused",
        progress: 0.25,
        animateProgress: true,
      }),
    );

    expect(activeHtml).toContain("role=\"img\"");
    expect(activeHtml).toContain("aria-label=\"Helper is enabled\"");
    expect(activeHtml).toContain("alt=\"Helper is enabled\"");
    expect(activeHtml).toContain("src=\"/Users/ChenYu/Documents/Github/clawpal/src/assets/doctor.png\"");
    expect(activeHtml).toContain("mx-auto w-[264px] sm:w-[312px]");
    expect(activeHtml).toContain("bg-[#78A287]");
    expect(activeHtml.match(/animate-pulse/g)?.length ?? 0).toBeGreaterThan(0);
    expect(activeHtml.match(/<span aria-hidden=\"true\"/g)?.length).toBe(14);
    expect(pausedHtml).toContain("bg-[#B38A54]");
    expect(pausedHtml.match(/<span aria-hidden=\"true\"/g)?.length).toBe(14);
    expect(activeHtml).not.toContain("<pre");
    expect(activeHtml).toContain("text-center");
  });
});
