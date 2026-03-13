import { describe, expect, spyOn, test } from "bun:test";

import { loadRecipeModelProfiles } from "../param-form-model-profiles";

describe("param-form-model-profiles", () => {
  test("loads recipe model profiles from the instance-bound loader", async () => {
    const api = {
      listRecipeModelProfiles: async () => [
        {
          id: "remote-openai",
          name: "Remote OpenAI",
          provider: "openai",
          model: "gpt-4o",
          authRef: "openai:default",
          enabled: true,
        },
        {
          id: "remote-disabled",
          name: "Remote Disabled",
          provider: "openai",
          model: "gpt-4.1",
          authRef: "openai:default",
          enabled: false,
        },
      ],
      listModelProfiles: async () => [
        {
          id: "local-openrouter",
          name: "Local OpenRouter",
          provider: "openrouter",
          model: "deepseek/deepseek-v3.2",
          authRef: "openrouter:default",
          enabled: true,
        },
      ],
    };
    const recipeSpy = spyOn(api, "listRecipeModelProfiles");
    const localSpy = spyOn(api, "listModelProfiles");

    await expect(loadRecipeModelProfiles(api)).resolves.toEqual([
      {
        id: "remote-openai",
        name: "Remote OpenAI",
        provider: "openai",
        model: "gpt-4o",
        authRef: "openai:default",
        enabled: true,
      },
    ]);
    expect(recipeSpy).toHaveBeenCalledTimes(1);
    expect(localSpy).not.toHaveBeenCalled();
  });
});
