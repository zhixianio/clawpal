import { describe, expect, test } from "bun:test";

import i18n from "@/i18n";
import {
  formatRecipeClaimForPeople,
  formatRecipeRunStatusLabel,
  resolveRecipeEnvironmentLabel,
} from "../recipe-run-copy";

describe("recipe run copy helpers", () => {
  test("formats recipe claims with user-facing labels", async () => {
    await i18n.changeLanguage("en");

    expect(
      formatRecipeClaimForPeople(i18n.t.bind(i18n), {
        kind: "agent",
        id: "support-bot",
      }),
    ).toBe("Agent: support-bot");

    expect(
      formatRecipeClaimForPeople(i18n.t.bind(i18n), {
        kind: "path",
        path: "~/.openclaw/config.json",
      }),
    ).toBe("Config or file: ~/.openclaw/config.json");
  });

  test("prefers friendly environment labels over raw instance ids", () => {
    expect(
      resolveRecipeEnvironmentLabel("ssh:recipe-docker", {
        currentInstanceId: "ssh:recipe-docker",
        currentInstanceLabel: "Recipe Docker",
        labelsById: {},
      }),
    ).toBe("Recipe Docker");

    expect(
      resolveRecipeEnvironmentLabel("docker:staging_lab", {
        currentInstanceId: "ssh:recipe-docker",
        currentInstanceLabel: "Recipe Docker",
        labelsById: {
          "docker:staging_lab": "Staging Lab",
        },
      }),
    ).toBe("Staging Lab");
  });

  test("maps run status to non-technical labels", async () => {
    await i18n.changeLanguage("en");

    expect(formatRecipeRunStatusLabel(i18n.t.bind(i18n), "succeeded")).toBe("Completed");
    expect(formatRecipeRunStatusLabel(i18n.t.bind(i18n), "failed")).toBe("Needs attention");
    expect(formatRecipeRunStatusLabel(i18n.t.bind(i18n), "running")).toBe("In progress");
  });
});
