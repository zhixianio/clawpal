import { describe, expect, test } from "bun:test";

import { getRecipeStudioProjectionDiff } from "@/lib/recipe-studio-projection-diff";

const CANONICAL_SOURCE = JSON.stringify({
  id: "channel-persona",
  name: "Channel Persona",
  description: "Apply a persona to one channel",
  version: "1.0.0",
  tags: ["discord"],
  difficulty: "easy",
  params: [],
  steps: [],
  bundle: {
    apiVersion: "strategy.platform/v1",
    kind: "StrategyBundle",
    metadata: {},
    compatibility: {},
    inputs: [],
    capabilities: { allowed: [] },
    resources: { supportedKinds: [] },
    execution: { supportedKinds: ["attachment"] },
    runner: {},
    outputs: [],
  },
  executionSpecTemplate: {
    apiVersion: "strategy.platform/v1",
    kind: "ExecutionSpec",
    metadata: {},
    source: {},
    target: {},
    execution: { kind: "attachment" },
    capabilities: { usedCapabilities: [] },
    resources: { claims: [] },
    secrets: { bindings: [] },
    desiredState: {},
    actions: [],
    outputs: [],
  },
}, null, 2);

describe("recipe studio projection diff", () => {
  test("treats canonical single-document source as in sync", () => {
    const diff = getRecipeStudioProjectionDiff(CANONICAL_SOURCE);

    expect(diff.hasDiff).toBe(false);
    expect(diff.affectedSections).toEqual([]);
  });

  test("flags wrapped recipe documents as source/form diffs", () => {
    const diff = getRecipeStudioProjectionDiff(JSON.stringify({
      recipes: [JSON.parse(CANONICAL_SOURCE)],
    }, null, 2));

    expect(diff.hasDiff).toBe(true);
    expect(diff.affectedSections).toContain("documentShape");
  });
});
