import { describe, expect, test } from "bun:test";

import { firstDroppedRecipeSource, isHttpRecipeSource } from "../recipe-source-input";

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

describe("firstDroppedRecipeSource", () => {
  test("returns the first non-empty dropped path", () => {
    expect(firstDroppedRecipeSource(["", "  ", "/tmp/recipe.json"])).toBe("/tmp/recipe.json");
  });

  test("returns null when every dropped path is blank", () => {
    expect(firstDroppedRecipeSource(["", "  "])).toBeNull();
  });
});
