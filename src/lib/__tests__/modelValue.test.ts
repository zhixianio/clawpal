import { describe, expect, test } from "bun:test";

import { profileToModelValue } from "../model-value";

describe("profileToModelValue", () => {
  test("combines provider and model", () => {
    expect(profileToModelValue({ provider: "anthropic", model: "claude-3-5-sonnet" }))
      .toBe("anthropic/claude-3-5-sonnet");
  });

  test("returns model alone when provider is empty", () => {
    expect(profileToModelValue({ provider: "", model: "gpt-4o" })).toBe("gpt-4o");
  });

  test("returns provider/ when model is empty", () => {
    expect(profileToModelValue({ provider: "openai", model: "" })).toBe("openai/");
  });

  test("does not double-prefix when model already has provider", () => {
    expect(profileToModelValue({ provider: "anthropic", model: "anthropic/claude-3-5-sonnet" }))
      .toBe("anthropic/claude-3-5-sonnet");
  });

  test("case-insensitive prefix detection", () => {
    expect(profileToModelValue({ provider: "Anthropic", model: "anthropic/claude-3-5-sonnet" }))
      .toBe("anthropic/claude-3-5-sonnet");
  });

  test("trims whitespace from provider and model", () => {
    expect(profileToModelValue({ provider: "  openai  ", model: "  gpt-4  " }))
      .toBe("openai/gpt-4");
  });

  test("handles both empty", () => {
    expect(profileToModelValue({ provider: "", model: "" })).toBe("");
  });

  test("whitespace-only provider treated as empty", () => {
    expect(profileToModelValue({ provider: "   ", model: "gpt-4o" })).toBe("gpt-4o");
  });
});
