import { describe, expect, test } from "bun:test";

import { getRecipeStudioActionState } from "@/lib/recipe-studio-actions";

describe("recipe studio action state", () => {
  test("blocks save, cook, and preview while validation is pending", () => {
    const state = getRecipeStudioActionState({
      source: "{\"id\":\"demo\"}",
      validating: true,
      validationError: null,
      diagnostics: {
        errors: [],
        warnings: [],
      },
      formSyncError: null,
      hasDraftRecipe: true,
    });

    expect(state.saveDisabled).toBe(true);
    expect(state.cookDisabled).toBe(true);
    expect(state.previewDisabled).toBe(true);
    expect(state.saveReasonKey).toBe("recipeStudio.validationBlockedValidating");
  });

  test("blocks actions when the draft has validation errors", () => {
    const state = getRecipeStudioActionState({
      source: "{\"id\":\"demo\"}",
      validating: false,
      validationError: null,
      diagnostics: {
        errors: [
          {
            category: "schema",
            severity: "error",
            message: "id is required",
          },
        ],
        warnings: [],
      },
      formSyncError: null,
      hasDraftRecipe: true,
    });

    expect(state.saveDisabled).toBe(true);
    expect(state.cookDisabled).toBe(true);
    expect(state.previewDisabled).toBe(true);
    expect(state.previewReasonKey).toBe("recipeStudio.validationBlockedFixErrors");
  });

  test("allows save but blocks cook and preview when no draft recipe can be derived", () => {
    const state = getRecipeStudioActionState({
      source: "{\"recipes\":[]}",
      validating: false,
      validationError: null,
      diagnostics: {
        errors: [],
        warnings: [],
      },
      formSyncError: null,
      hasDraftRecipe: false,
    });

    expect(state.saveDisabled).toBe(false);
    expect(state.cookDisabled).toBe(true);
    expect(state.previewDisabled).toBe(true);
    expect(state.cookReasonKey).toBe("recipeStudio.validationBlockedNoRecipe");
  });
});
