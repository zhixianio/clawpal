import { describe, expect, test } from "bun:test";

import { isHttpRecipeSource } from "../recipe-source-input";

describe("isHttpRecipeSource", () => {
  test("accepts http and https recipe URLs", () => {
    expect(isHttpRecipeSource("https://example.com/recipe.json")).toBe(true);
    expect(isHttpRecipeSource("  http://example.com/recipes.json  ")).toBe(true);
  });

  test("rejects local file and library paths", () => {
    expect(isHttpRecipeSource("/tmp/recipes.json")).toBe(false);
    expect(isHttpRecipeSource("/tmp/recipe-library")).toBe(false);
    expect(isHttpRecipeSource("recipe-library")).toBe(false);
  });
});

