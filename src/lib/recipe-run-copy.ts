import type { TFunction } from "i18next";

import type { ExecutionResourceClaim } from "./types";

type EnvironmentLabelContext = {
  currentInstanceId: string;
  currentInstanceLabel?: string | null;
  labelsById?: Record<string, string>;
};

function humanizeEnvironmentFallback(instanceId: string): string {
  const normalized = instanceId
    .replace(/^ssh:/, "")
    .replace(/^docker:/, "")
    .replace(/^local$/, "Local")
    .replace(/[-_]+/g, " ")
    .trim();

  if (!normalized) {
    return "Local";
  }

  return normalized.replace(/\b\w/g, (char) => char.toUpperCase());
}

export function resolveRecipeEnvironmentLabel(
  instanceId: string,
  context: EnvironmentLabelContext,
): string {
  if (
    instanceId === context.currentInstanceId &&
    typeof context.currentInstanceLabel === "string" &&
    context.currentInstanceLabel.trim().length > 0
  ) {
    return context.currentInstanceLabel.trim();
  }

  const mapped = context.labelsById?.[instanceId];
  if (typeof mapped === "string" && mapped.trim().length > 0) {
    return mapped.trim();
  }

  return humanizeEnvironmentFallback(instanceId);
}

export function formatRecipeClaimForPeople(
  t: TFunction,
  claim: ExecutionResourceClaim,
): string {
  const value = [claim.id, claim.target, claim.path].filter(Boolean).join(" · ");
  const fallback = t("cook.reviewGenericResource");
  const detail = value || fallback;

  switch (claim.kind) {
    case "agent":
      return t("cook.reviewClaimAgent", { value: detail });
    case "channel":
      return t("cook.reviewClaimChannel", { value: detail });
    case "file":
    case "path":
      return t("cook.reviewClaimFile", { value: detail });
    case "document":
      return t("cook.reviewClaimDocument", { value: detail });
    case "modelProfile":
      return t("cook.reviewClaimModelProfile", { value: detail });
    case "authProfile":
      return t("cook.reviewClaimAuthProfile", { value: detail });
    case "service":
      return t("cook.reviewClaimService", { value: detail });
    default:
      return t("cook.reviewClaimGeneric", { kind: claim.kind, value: detail });
  }
}

export function formatRecipeRunStatusLabel(t: TFunction, status: string): string {
  if (status === "succeeded") {
    return t("orchestrator.statusCompleted");
  }
  if (status === "failed") {
    return t("orchestrator.statusNeedsAttention");
  }
  return t("orchestrator.statusInProgress");
}
