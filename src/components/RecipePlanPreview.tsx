import { useState } from "react";
import { ChevronDownIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import type { PrecheckIssue, RecipePlan, RecipeWorkspaceEntry } from "@/lib/types";
import type { CookRouteSummary } from "@/pages/cook-plan-context";

function formatIssue(issue: PrecheckIssue) {
  return `${issue.code}: ${issue.message}`;
}

export function RecipePlanPreview({
  plan,
  routeSummary,
  authIssues = [],
  contextWarnings = [],
  workspaceEntry = null,
}: {
  plan: RecipePlan;
  routeSummary?: CookRouteSummary;
  authIssues?: PrecheckIssue[];
  contextWarnings?: string[];
  workspaceEntry?: RecipeWorkspaceEntry | null;
}) {
  const { t } = useTranslation();
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const hasBlockingAuthIssue = authIssues.some((issue) => issue.severity === "error");
  const approvalRequired = Boolean(workspaceEntry?.approvalRequired);
  const combinedWarnings = [...plan.warnings, ...contextWarnings];
  const plannedActions = plan.executionSpec.actions.map((action, index) => ({
    id: `${action.kind ?? "action"}-${index}`,
    label:
      (typeof action.name === "string" ? action.name.trim() : "") ||
      t("cook.reviewDefaultAction", { index: index + 1 }),
  }));

  const formattedClaims = plan.concreteClaims.map((claim, index) => {
    const value = [claim.id, claim.target, claim.path].filter(Boolean).join(" · ");
    const fallback = t("cook.reviewGenericResource");
    const detail = value || fallback;
    const key = `${claim.kind}-${claim.id ?? claim.target ?? claim.path ?? index}`;

    switch (claim.kind) {
      case "agent":
        return { key, label: t("cook.reviewClaimAgent", { value: detail }) };
      case "channel":
        return { key, label: t("cook.reviewClaimChannel", { value: detail }) };
      case "file":
      case "path":
        return { key, label: t("cook.reviewClaimFile", { value: detail }) };
      case "document":
        return { key, label: t("cook.reviewClaimDocument", { value: detail }) };
      case "modelProfile":
        return { key, label: t("cook.reviewClaimModelProfile", { value: detail }) };
      case "authProfile":
        return { key, label: t("cook.reviewClaimAuthProfile", { value: detail }) };
      case "service":
        return { key, label: t("cook.reviewClaimService", { value: detail }) };
      default:
        return { key, label: t("cook.reviewClaimGeneric", { kind: claim.kind, value: detail }) };
    }
  });
  const blockingItems = [
    ...authIssues
      .filter((issue) => issue.severity === "error")
      .map((issue) => ({
        key: `auth:${issue.code}:${issue.message}`,
        summary: issue.message,
        detail: formatIssue(issue),
      })),
    ...(approvalRequired
      ? [
          {
            key: "approval-required",
            summary: t("cook.reviewApprovalBlocker"),
            detail: t("cook.reviewApprovalBlockerDetail"),
          },
        ]
      : []),
  ];
  const attentionItems = [
    ...authIssues
      .filter((issue) => issue.severity !== "error")
      .map((issue) => ({
      key: `auth:${issue.code}:${issue.message}`,
      summary: issue.message,
      detail: formatIssue(issue),
    })),
    ...combinedWarnings.map((warning) => ({
      key: `warning:${warning}`,
      summary: warning,
      detail: warning,
    })),
  ];
  const routeKindLabel =
    routeSummary?.kind === "ssh"
      ? t("cook.routeKindSsh")
      : routeSummary?.kind === "docker"
        ? t("cook.routeKindDocker")
        : t("cook.routeKindLocal");

  return (
    <div className="mb-4 rounded-lg border border-border/70 bg-muted/20 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-medium">{plan.summary.recipeName}</div>
          <div className="text-xs text-muted-foreground">
            {t("cook.reviewSummary", {
              count: plan.summary.actionCount,
            })}
            {plan.summary.skippedStepCount > 0
              ? ` · ${t("cook.reviewSkippedSummary", {
                count: plan.summary.skippedStepCount,
              })}`
              : ""}
          </div>
        </div>
        <div className="rounded-full border border-border/70 bg-background/80 px-3 py-1 text-xs text-muted-foreground">
          {t("cook.reviewExecutionKind", {
            kind: plan.summary.executionKind,
          })}
        </div>
      </div>

      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <div className="rounded-md border border-border/70 bg-background/80 p-3">
          <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
            {t("cook.reviewWhatTitle")}
          </div>
          <ul className="mt-2 space-y-2 text-sm text-foreground">
            {plannedActions.map((action) => (
              <li key={action.id} className="flex gap-2">
                <span className="mt-0.5 text-muted-foreground">•</span>
                <span>{action.label}</span>
              </li>
            ))}
          </ul>
          {plan.summary.skippedStepCount > 0 && (
            <div className="mt-3 rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
              {t("cook.reviewSkippedHint", { count: plan.summary.skippedStepCount })}
            </div>
          )}
        </div>

        <div className="rounded-md border border-border/70 bg-background/80 p-3">
          <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
            {t("cook.reviewWhereTitle")}
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-sm text-foreground">
            {routeSummary ? (
              <>
                <Badge variant="outline">{routeKindLabel}</Badge>
                <span>{routeSummary.targetLabel}</span>
              </>
            ) : (
              <span className="text-muted-foreground">{t("cook.reviewDefaultRoute")}</span>
            )}
          </div>
        </div>
      </div>

      <div className="mt-4 rounded-md border border-border/70 bg-background/80 p-3">
        <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
          {t("cook.reviewResourcesTitle")}
        </div>
        <ul className="mt-2 space-y-2 text-sm text-foreground">
          {formattedClaims.map((claim) => (
            <li key={claim.key} className="flex gap-2">
              <span className="mt-0.5 text-muted-foreground">•</span>
              <span>{claim.label}</span>
            </li>
          ))}
        </ul>
      </div>

      <div className="mt-4 rounded-md border border-border/70 bg-background/80 p-3">
        <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
          {t("cook.reviewRequirementsTitle")}
        </div>
        <ul className="mt-2 space-y-2 text-sm text-foreground">
          <li className="flex gap-2">
            <span className="mt-0.5 text-muted-foreground">•</span>
            <span>{t("cook.reviewRequirementEnvironment", { name: routeSummary?.targetLabel ?? t("cook.reviewDefaultRoute") })}</span>
          </li>
          {approvalRequired && (
            <li className="flex gap-2">
              <span className="mt-0.5 text-muted-foreground">•</span>
              <span>{t("cook.reviewRequirementApproval")}</span>
            </li>
          )}
          {!approvalRequired && !hasBlockingAuthIssue && (
            <li className="flex gap-2">
              <span className="mt-0.5 text-muted-foreground">•</span>
              <span>{t("cook.reviewRequirementReady")}</span>
            </li>
          )}
        </ul>
      </div>

      {blockingItems.length > 0 && (
        <div className="mt-4 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          <div className="font-medium">{t("cook.reviewBlockingTitle")}</div>
          <div className="mt-2 space-y-2">
            {blockingItems.map((item) => (
              <div key={item.key}>
                <div>{item.summary}</div>
                {item.detail !== item.summary && (
                  <div className="mt-1 text-xs opacity-80">{item.detail}</div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {attentionItems.length > 0 && (
        <div
          className="mt-4 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-950"
        >
          <div className="font-medium">{t("cook.reviewAttentionTitle")}</div>
          <div className="mt-2 space-y-2">
            {attentionItems.map((item) => (
              <div key={item.key}>
                <div>{item.summary}</div>
                {item.detail !== item.summary && (
                  <div className="mt-1 text-xs opacity-80">{item.detail}</div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      <details
        className="mt-4 rounded-md border border-border/70 bg-background/80 px-3 py-2"
        onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
      >
        <summary className="flex cursor-pointer list-none items-center justify-between gap-3 text-sm font-medium text-foreground">
          <span>{t("cook.reviewAdvancedTitle")}</span>
          <ChevronDownIcon
            className={`size-4 text-muted-foreground transition-transform ${
              advancedOpen ? "rotate-180" : ""
            }`}
            aria-hidden="true"
          />
        </summary>
        <div className="mt-3 space-y-3">
          <div>
            <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
              {t("cook.reviewPlanReference")}
            </div>
            <div className="mt-1 font-mono text-xs">{plan.executionSpecDigest}</div>
          </div>
          <div>
            <div className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
              {t("cook.reviewRequiredAccess")}
            </div>
            <div className="mt-2 flex flex-wrap gap-2">
              {plan.usedCapabilities.map((capability) => (
                <span
                  key={capability}
                  className="rounded-full bg-muted px-2.5 py-1 font-mono text-xs text-muted-foreground"
                >
                  {capability}
                </span>
              ))}
            </div>
          </div>
        </div>
      </details>
    </div>
  );
}
