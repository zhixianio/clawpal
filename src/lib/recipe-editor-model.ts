import * as JSON5 from "json5";

import type {
  Recipe,
  RecipeEditorActionRow,
  RecipeEditorModel,
  RecipeExecutionKind,
  RecipeParam,
  RecipeStep,
} from "@/lib/types";

type RecipeSourceDocument = Recipe & {
  bundle?: {
    capabilities?: { allowed?: string[] };
    resources?: { supportedKinds?: string[] };
  };
  executionSpecTemplate?: {
    kind?: string;
    execution?: { kind?: string };
    actions?: Array<{
      kind?: string;
      name?: string;
      args?: Record<string, unknown>;
    }>;
  };
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function isRecipeSourceDocument(value: unknown): value is RecipeSourceDocument {
  const record = asRecord(value);
  return !!record
    && typeof record.id === "string"
    && typeof record.name === "string"
    && Array.isArray(record.params)
    && Array.isArray(record.steps);
}

export function parseRecipeSource(source: string): RecipeSourceDocument {
  const parsed = JSON5.parse(source) as unknown;
  const candidates = Array.isArray(parsed)
    ? parsed
    : Array.isArray(asRecord(parsed)?.recipes)
      ? (asRecord(parsed)?.recipes as unknown[])
      : [parsed];
  const doc = candidates.find(isRecipeSourceDocument);
  if (!doc) {
    throw new Error("source does not contain a structured recipe document");
  }
  return doc;
}

function prettyArgs(value: unknown): string {
  return JSON.stringify(value ?? {}, null, 2);
}

function parseArgsText(text: string): Record<string, unknown> {
  if (!text.trim()) {
    return {};
  }
  return JSON5.parse(text) as Record<string, unknown>;
}

function cloneParams(params: RecipeParam[]): RecipeParam[] {
  return params.map((param) => ({ ...param }));
}

function cloneSteps(steps: RecipeStep[]): RecipeStep[] {
  return steps.map((step) => ({
    ...step,
    args: { ...step.args },
  }));
}

function cloneDocument(doc: RecipeSourceDocument): RecipeSourceDocument {
  return JSON.parse(JSON.stringify(doc)) as RecipeSourceDocument;
}

function normalizeExecutionKind(value: string | undefined): RecipeExecutionKind {
  switch (value) {
    case "job":
    case "service":
    case "schedule":
    case "attachment":
      return value;
    default:
      return "attachment";
  }
}

export function toRecipeEditorModel(doc: RecipeSourceDocument): RecipeEditorModel {
  return {
    id: doc.id,
    name: doc.name,
    description: doc.description,
    version: doc.version,
    tagsText: doc.tags.join(", "),
    difficulty: doc.difficulty,
    params: cloneParams(doc.params),
    steps: cloneSteps(doc.steps),
    actionRows: (doc.executionSpecTemplate?.actions ?? []).map(
      (action): RecipeEditorActionRow => ({
        kind: action.kind ?? "",
        name: action.name ?? "",
        argsText: prettyArgs(action.args ?? {}),
      }),
    ),
    bundleCapabilities: [...(doc.bundle?.capabilities?.allowed ?? [])],
    bundleResources: [...(doc.bundle?.resources?.supportedKinds ?? [])],
    executionKind: normalizeExecutionKind(doc.executionSpecTemplate?.execution?.kind),
    sourceDocument: cloneDocument(doc) as unknown,
  };
}

export function fromRecipeEditorModel(model: RecipeEditorModel): RecipeSourceDocument {
  const nextDoc = cloneDocument(model.sourceDocument as RecipeSourceDocument);
  nextDoc.id = model.id;
  nextDoc.name = model.name;
  nextDoc.description = model.description;
  nextDoc.version = model.version;
  nextDoc.tags = model.tagsText
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);
  nextDoc.difficulty = model.difficulty;
  nextDoc.params = cloneParams(model.params);
  nextDoc.steps = cloneSteps(model.steps);

  if (!nextDoc.bundle) {
    nextDoc.bundle = {};
  }
  if (!nextDoc.bundle.capabilities) {
    nextDoc.bundle.capabilities = {};
  }
  if (!nextDoc.bundle.resources) {
    nextDoc.bundle.resources = {};
  }
  nextDoc.bundle.capabilities.allowed = [...model.bundleCapabilities];
  nextDoc.bundle.resources.supportedKinds = [...model.bundleResources];

  if (!nextDoc.executionSpecTemplate) {
    nextDoc.executionSpecTemplate = {};
  }
  nextDoc.executionSpecTemplate.kind = nextDoc.executionSpecTemplate.kind ?? "ExecutionSpec";
  nextDoc.executionSpecTemplate.execution = {
    ...(nextDoc.executionSpecTemplate.execution ?? {}),
    kind: model.executionKind,
  };
  nextDoc.executionSpecTemplate.actions = model.actionRows.map((row) => ({
    kind: row.kind || undefined,
    name: row.name || undefined,
    args: parseArgsText(row.argsText),
  }));

  return nextDoc;
}

export function serializeRecipeEditorModel(model: RecipeEditorModel): string {
  return JSON.stringify(fromRecipeEditorModel(model), null, 2);
}
