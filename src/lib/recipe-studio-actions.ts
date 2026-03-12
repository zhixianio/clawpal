import type { RecipeSourceDiagnostics } from "@/lib/types";

type RecipeStudioActionStateInput = {
  source: string;
  validating: boolean;
  validationError: string | null;
  diagnostics: RecipeSourceDiagnostics;
  formSyncError: string | null;
  hasDraftRecipe: boolean;
};

type RecipeStudioActionState = {
  saveDisabled: boolean;
  saveReasonKey: string | null;
  cookDisabled: boolean;
  cookReasonKey: string | null;
  previewDisabled: boolean;
  previewReasonKey: string | null;
};

function getRecipeStudioValidationReasonKey({
  source,
  validating,
  validationError,
  diagnostics,
  formSyncError,
}: Omit<RecipeStudioActionStateInput, "hasDraftRecipe">): string | null {
  if (!source.trim()) {
    return "recipeStudio.validationBlockedEmpty";
  }
  if (validating) {
    return "recipeStudio.validationBlockedValidating";
  }
  if (validationError) {
    return "recipeStudio.validationBlockedUnavailable";
  }
  if (formSyncError) {
    return "recipeStudio.validationBlockedFormSync";
  }
  if (diagnostics.errors.length > 0) {
    return "recipeStudio.validationBlockedFixErrors";
  }
  return null;
}

export function getRecipeStudioActionState(
  input: RecipeStudioActionStateInput,
): RecipeStudioActionState {
  const validationReasonKey = getRecipeStudioValidationReasonKey(input);
  const saveDisabled = validationReasonKey !== null;
  const executionReasonKey = validationReasonKey
    ?? (input.hasDraftRecipe ? null : "recipeStudio.validationBlockedNoRecipe");

  return {
    saveDisabled,
    saveReasonKey: validationReasonKey,
    cookDisabled: executionReasonKey !== null,
    cookReasonKey: executionReasonKey,
    previewDisabled: executionReasonKey !== null,
    previewReasonKey: executionReasonKey,
  };
}
