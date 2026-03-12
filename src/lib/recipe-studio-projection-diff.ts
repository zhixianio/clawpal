import * as JSON5 from "json5";

import {
  fromRecipeEditorModel,
  parseRecipeSource,
  toRecipeEditorModel,
} from "@/lib/recipe-editor-model";

export type RecipeStudioProjectionDiff = {
  hasDiff: boolean;
  affectedSections: string[];
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function isSingleRecipeDocumentRoot(value: unknown): boolean {
  const record = asRecord(value);
  return !!record
    && typeof record.id === "string"
    && typeof record.name === "string"
    && Array.isArray(record.params)
    && Array.isArray(record.steps);
}

function normalize(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(normalize);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, normalize(nested)]),
    );
  }
  return value;
}

function isEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(normalize(left)) === JSON.stringify(normalize(right));
}

function collectAffectedSections(
  sourceDoc: Record<string, unknown>,
  canonicalDoc: Record<string, unknown>,
): string[] {
  const keys = new Set([
    ...Object.keys(sourceDoc),
    ...Object.keys(canonicalDoc),
  ]);
  return Array.from(keys).filter((key) => !isEqual(sourceDoc[key], canonicalDoc[key]));
}

export function getRecipeStudioProjectionDiff(source: string): RecipeStudioProjectionDiff {
  try {
    const parsed = JSON5.parse(source) as unknown;
    const recipeDoc = parseRecipeSource(source);
    const sourceDoc = asRecord(recipeDoc);
    const canonicalDoc = asRecord(
      fromRecipeEditorModel(toRecipeEditorModel(recipeDoc)),
    );
    if (!sourceDoc || !canonicalDoc) {
      return {
        hasDiff: false,
        affectedSections: [],
      };
    }

    const affectedSections = new Set<string>();
    if (!isSingleRecipeDocumentRoot(parsed)) {
      affectedSections.add("documentShape");
    }
    for (const key of collectAffectedSections(sourceDoc, canonicalDoc)) {
      affectedSections.add(key);
    }

    return {
      hasDiff: affectedSections.size > 0,
      affectedSections: Array.from(affectedSections),
    };
  } catch {
    return {
      hasDiff: false,
      affectedSections: [],
    };
  }
}
