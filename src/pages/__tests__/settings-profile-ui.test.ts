import { describe, expect, test } from "bun:test";

import type { ModelProfile } from "@/lib/types";
import { getSettingsProfileUiState } from "../settings-profile-ui";

describe("settings-profile-ui", () => {
  test("hides test and disable controls while keeping edit and delete available", () => {
    const profile: ModelProfile = {
      id: "profile-1",
      name: "Primary OpenAI",
      provider: "openai",
      model: "gpt-5",
      authRef: "OPENAI_API_KEY",
      enabled: true,
    };

    expect(getSettingsProfileUiState(profile)).toEqual({
      showEnabledBadge: false,
      showEnabledToggle: false,
      actions: ["edit", "delete"],
    });
  });
});
