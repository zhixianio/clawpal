import { describe, expect, test } from "bun:test";

import {
  buildCookExecuteRequest,
  buildCookExecutionSpec,
  buildCookPhaseItems,
  getCookExecutionProgress,
  getCookPlanningProgress,
  markCookFailure,
  markCookStatuses,
} from "../cook-execution";

describe("cook execution helpers", () => {
  test("builds a remote execution target from instance context", () => {
    const spec = buildCookExecutionSpec(
      {
        apiVersion: "strategy.platform/v1",
        kind: "ExecutionSpec",
        metadata: {},
        source: {},
        target: {},
        execution: { kind: "job" },
        capabilities: { usedCapabilities: [] },
        resources: { claims: [] },
        secrets: { bindings: [] },
        desiredState: {},
        actions: [],
        outputs: [],
      },
      {
        instanceId: "ssh:prod-a",
        isRemote: true,
        isDocker: false,
      },
    );

    expect(spec.target).toEqual({
      kind: "remote_ssh",
      hostId: "ssh:prod-a",
    });
  });

  test("marks non-skipped steps with the requested execution state", () => {
    expect(markCookStatuses(["pending", "skipped", "pending"], "running")).toEqual([
      "running",
      "skipped",
      "running",
    ]);
  });

  test("restores running steps to pending when execution fails", () => {
    expect(markCookFailure(["running", "done", "skipped"])).toEqual([
      "pending",
      "done",
      "skipped",
    ]);
  });

  test("builds phase items for the done screen", () => {
    expect(buildCookPhaseItems("done")).toEqual([
      { key: "params", labelKey: "cook.phaseConfigure", state: "complete" },
      { key: "confirm", labelKey: "cook.phaseReview", state: "complete" },
      { key: "execute", labelKey: "cook.phaseExecute", state: "complete" },
      { key: "done", labelKey: "cook.phaseDone", state: "current" },
    ]);
  });

  test("builds a cook execute request that preserves draft origin", () => {
    const request = buildCookExecuteRequest(
      {
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
      {
        instanceId: "local",
        isRemote: false,
        isDocker: false,
      },
      "draft",
      "{\n  \"id\": \"draft\"\n}",
      "channel-persona",
    );

    expect(request.sourceOrigin).toBe("draft");
    expect(request.sourceText).toContain("\"id\": \"draft\"");
    expect(request.workspaceSlug).toBe("channel-persona");
    expect(request.spec.target).toEqual({ kind: "local" });
  });

  test("maps planning stages to determinate progress values", () => {
    expect(getCookPlanningProgress("validate")).toEqual({
      value: 20,
      labelKey: "cook.progressValidate",
    });
    expect(getCookPlanningProgress("build")).toEqual({
      value: 70,
      labelKey: "cook.progressBuild",
    });
    expect(getCookPlanningProgress("checks")).toEqual({
      value: 100,
      labelKey: "cook.progressChecks",
    });
  });

  test("uses operation-level execution progress while a recipe is applying", () => {
    expect(getCookExecutionProgress("running", ["pending", "pending", "skipped"])).toEqual({
      value: 65,
      actionableCount: 2,
      totalCount: 3,
      failed: false,
      animated: true,
      detailKey: "cook.executionApplyingDetail",
      detailArgs: {
        actionable: 2,
        total: 3,
      },
    });
  });

  test("reports a failed execution without pretending every step failed", () => {
    expect(getCookExecutionProgress("failed", ["pending", "pending", "skipped"])).toEqual({
      value: 65,
      actionableCount: 2,
      totalCount: 3,
      failed: true,
      animated: false,
      detailKey: "cook.executionFailedDetail",
      detailArgs: {
        actionable: 2,
        total: 3,
      },
    });
  });

  test("reports a completed execution at 100 percent", () => {
    expect(getCookExecutionProgress("done", ["done", "done", "skipped"])).toEqual({
      value: 100,
      actionableCount: 2,
      totalCount: 3,
      failed: false,
      animated: false,
      detailKey: "cook.executionDoneDetail",
      detailArgs: {
        complete: 2,
        total: 3,
      },
    });
  });
});
