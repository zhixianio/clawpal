import type { ExecuteRecipeRequest, ExecutionSpec, RecipeSourceOrigin } from "@/lib/types";

export type CookStepStatus = "pending" | "running" | "done" | "failed" | "skipped";

export interface CookExecutionContext {
  instanceId: string;
  isRemote: boolean;
  isDocker: boolean;
}

export function buildCookExecutionSpec(
  spec: ExecutionSpec,
  context: CookExecutionContext,
): ExecutionSpec {
  const target = context.isRemote
    ? { kind: "remote_ssh", hostId: context.instanceId }
    : { kind: context.isDocker ? "docker_local" : "local" };

  return {
    ...spec,
    target,
  };
}

export function buildCookExecuteRequest(
  spec: ExecutionSpec,
  context: CookExecutionContext,
  sourceOrigin: RecipeSourceOrigin,
  sourceText?: string,
  workspaceSlug?: string,
): ExecuteRecipeRequest {
  return {
    spec: buildCookExecutionSpec(spec, context),
    sourceOrigin,
    sourceText,
    workspaceSlug,
  };
}

export function markCookStatuses(
  statuses: CookStepStatus[],
  next: Exclude<CookStepStatus, "skipped">,
): CookStepStatus[] {
  return statuses.map((status) => (status === "skipped" ? "skipped" : next));
}

export function markCookFailure(statuses: CookStepStatus[]): CookStepStatus[] {
  let failed = false;
  return statuses.map((status) => {
    if (status === "skipped") return "skipped";
    if (!failed) {
      failed = true;
      return "failed";
    }
    return "pending";
  });
}
