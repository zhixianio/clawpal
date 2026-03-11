import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { RecipePlanPreview } from "../RecipePlanPreview";

describe("RecipePlanPreview", () => {
  test("renders capability and resource summaries in the confirm phase", () => {
    const html = renderToStaticMarkup(
      React.createElement(RecipePlanPreview, {
        plan: {
          summary: {
            recipeId: "discord-channel-persona",
            recipeName: "Channel Persona",
            executionKind: "attachment",
            actionCount: 1,
            skippedStepCount: 0,
          },
          usedCapabilities: ["service.manage"],
          concreteClaims: [{ kind: "path", path: "~/.openclaw/config.json" }],
          executionSpecDigest: "digest-123",
          warnings: [],
        },
      }),
    );

    expect(html).toContain("service.manage");
    expect(html).toContain("path");
    expect(html).toContain("digest-123");
  });
});
