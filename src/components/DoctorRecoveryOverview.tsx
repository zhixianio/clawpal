import { WrenchIcon } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type {
  RescuePrimaryDiagnosisResult,
  RescuePrimaryRepairResult,
  RescuePrimarySectionItem,
} from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  localizeDoctorReportText,
  localizeRescuePrimaryDiagnosis,
} from "@/lib/doctor-report-i18n";
import { cn } from "@/lib/utils";

interface DoctorRecoveryOverviewProps {
  diagnosis: RescuePrimaryDiagnosisResult;
  checkLoading: boolean;
  repairing: boolean;
  progressLine: string | null;
  repairResult: RescuePrimaryRepairResult | null;
  repairError: string | null;
  onRepairAll: () => void;
  onRepairIssue: (issueId: string) => void;
  showRepairActions?: boolean;
}

function hasDocGuidance(target: {
  rootCauseHypotheses?: { title: string; reason: string }[];
  fixSteps?: string[];
  citations?: { url: string; section: string }[];
  versionAwareness?: string;
}) {
  return !!(
    target.rootCauseHypotheses?.length
    || target.fixSteps?.length
    || target.versionAwareness
  );
}

function itemBadgeVariant(status: RescuePrimarySectionItem["status"]) {
  return status === "error" ? "destructive" : "outline";
}

function statusBadgeClass(
  status: RescuePrimaryDiagnosisResult["status"] | RescuePrimarySectionItem["status"],
) {
  if (status === "healthy" || status === "ok") {
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:border-emerald-400/30 dark:bg-emerald-400/10 dark:text-emerald-300";
  }
  if (status === "degraded" || status === "warn") {
    return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:border-amber-400/30 dark:bg-amber-400/10 dark:text-amber-300";
  }
  if (status === "inactive" || status === "info") {
    return "border-border/60 bg-muted/30 text-muted-foreground";
  }
  return "";
}

function shouldShowSectionStatusBadge(
  summaryStatus: RescuePrimaryDiagnosisResult["status"],
  sectionStatus: RescuePrimaryDiagnosisResult["status"],
) {
  if (sectionStatus === "healthy" || sectionStatus === "inactive") {
    return true;
  }
  return sectionStatus !== summaryStatus;
}

function shouldShowItemStatusBadge(
  sectionStatus: RescuePrimaryDiagnosisResult["status"],
  itemStatus: RescuePrimarySectionItem["status"],
) {
  if (itemStatus === "ok" || itemStatus === "info") {
    return true;
  }
  if (sectionStatus === "broken" && itemStatus === "error") {
    return false;
  }
  if (sectionStatus === "degraded" && itemStatus === "warn") {
    return false;
  }
  return true;
}

export function DoctorRecoveryOverview({
  diagnosis,
  checkLoading,
  repairing,
  progressLine,
  repairResult,
  repairError,
  onRepairAll,
  onRepairIssue,
  showRepairActions = true,
}: DoctorRecoveryOverviewProps) {
  const { t, i18n } = useTranslation();
  const viewDiagnosis = useMemo(
    () => localizeRescuePrimaryDiagnosis(diagnosis, i18n.language),
    [diagnosis, i18n.language],
  );
  const fixableCount = viewDiagnosis.summary.fixableIssueCount;
  const isOptimizeIntent = viewDiagnosis.summary.status === "degraded";
  const fixText = isOptimizeIntent
    ? t(
      fixableCount === 1 ? "doctor.optimizeOneIssue" : "doctor.optimizeManyIssues",
      {
        count: fixableCount,
        defaultValue: fixableCount === 1 ? "Optimize 1 issue" : `Optimize ${fixableCount} issues`,
      },
    )
    : t("doctor.fixSafeIssues", {
      count: fixableCount,
      defaultValue: fixableCount === 1 ? "Fix 1 issue" : `Fix ${fixableCount} issues`,
    });
  const summaryHypothesis = viewDiagnosis.summary.rootCauseHypotheses?.[0] ?? null;
  const summaryFixSteps = useMemo(
    () => (viewDiagnosis.summary.fixSteps ?? []).slice(0, 3),
    [viewDiagnosis.summary.fixSteps],
  );
  const viewProgressLine = useMemo(
    () => (progressLine ? localizeDoctorReportText(progressLine, i18n.language) : null),
    [i18n.language, progressLine],
  );
  const showSummaryCard = viewDiagnosis.summary.status !== "healthy";
  const visibleSections = useMemo(
    () => viewDiagnosis.sections
      .map((section) => {
        if (section.key !== "gateway") {
          return section;
        }
        return {
          ...section,
          items: section.items.filter((item) => item.issueId !== "primary.gateway.unhealthy"),
        };
      })
      .filter((section) => section.items.length > 0),
    [viewDiagnosis.sections],
  );
  const affectedSections = useMemo(
    () => visibleSections.filter((section) => section.status !== "healthy"),
    [visibleSections],
  );
  const translateStatus = (
    status: RescuePrimaryDiagnosisResult["status"] | RescuePrimarySectionItem["status"],
  ) => {
    if (status === "healthy" || status === "ok") {
      return t("doctor.primaryStatusHealthy", { defaultValue: "Healthy" });
    }
    if (status === "degraded" || status === "warn") {
      return t("doctor.primaryStatusDegraded", { defaultValue: "Degraded" });
    }
    if (status === "broken" || status === "error") {
      return t("doctor.primaryStatusBroken", { defaultValue: "Broken" });
    }
    if (status === "inactive") {
      return t("doctor.primaryStatusInactive", { defaultValue: "Inactive" });
    }
    return status;
  };

  return (
    <div className="mt-4 space-y-4">
      {showSummaryCard ? (
        <Card className="border-border/60 bg-muted/20">
        <CardHeader className={cn("pb-3", !showRepairActions && "pb-2")}>
          <div className="flex items-start justify-between gap-3">
            <div className="space-y-1">
              <CardTitle className="text-base">{viewDiagnosis.summary.headline}</CardTitle>
              <p className="text-sm text-muted-foreground">
                {viewDiagnosis.summary.recommendedAction}
              </p>
            </div>
            <Badge
              variant={viewDiagnosis.summary.status === "broken" ? "destructive" : "outline"}
              className={statusBadgeClass(viewDiagnosis.summary.status)}
            >
              {translateStatus(viewDiagnosis.summary.status)}
            </Badge>
          </div>
        </CardHeader>
        <CardContent className={cn("space-y-3", !showRepairActions && "space-y-2 pt-0")}>
          <div className={cn("flex flex-wrap items-center gap-2", !showRepairActions && "justify-between gap-3")}>
            {showRepairActions ? (
              <Button
                size="sm"
                onClick={onRepairAll}
                disabled={checkLoading || repairing || fixableCount === 0}
              >
                <WrenchIcon className="mr-1.5 size-3.5" />
                {fixText}
              </Button>
            ) : null}
            {!showRepairActions && affectedSections.length > 0 ? (
              <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                {affectedSections.map((section) => (
                  <span
                    key={section.key}
                    className="rounded-full border border-border/60 bg-background/80 px-2 py-1"
                  >
                    {section.title}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
          {viewProgressLine ? (
            <div className="h-5 overflow-hidden text-sm text-muted-foreground">
              <span
                key={viewProgressLine}
                className="inline-block whitespace-nowrap transition-opacity duration-300 animate-pulse"
              >
                {viewProgressLine}
              </span>
            </div>
          ) : null}
          {repairResult ? (
            <div className="text-sm text-muted-foreground">
              {t(
                isOptimizeIntent ? "doctor.optimizeSummaryInline" : "doctor.repairSummaryInline",
                {
                  defaultValue: isOptimizeIntent
                    ? "Optimized {{applied}} issue(s), skipped {{skipped}}, failed {{failed}}."
                    : "Fixed {{applied}} issue(s), skipped {{skipped}}, failed {{failed}}.",
                applied: repairResult.appliedIssueIds.length,
                skipped: repairResult.skippedIssueIds.length,
                failed: repairResult.failedIssueIds.length,
                },
              )}
            </div>
          ) : null}
          {repairError ? (
            <div className="text-sm text-destructive">{repairError}</div>
          ) : null}
          {showRepairActions && affectedSections.length > 0 ? (
            <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
              {affectedSections.map((section) => (
                <span
                  key={section.key}
                  className="rounded-full border border-border/60 bg-background/80 px-2 py-1"
                >
                  {section.title}
                </span>
              ))}
            </div>
          ) : null}
          {hasDocGuidance(viewDiagnosis.summary) ? (
            <div className="rounded-md border border-border/60 bg-background/70 p-3 text-sm">
              <div className="space-y-2">
                {summaryHypothesis ? (
                  <div>
                    <div className="font-medium">{summaryHypothesis.title}</div>
                    <div className="text-muted-foreground">{summaryHypothesis.reason}</div>
                  </div>
                ) : null}
                {summaryFixSteps.length ? (
                  <div className="space-y-1">
                    {summaryFixSteps.map((step, index) => (
                      <div key={`${step}-${index}`} className="text-muted-foreground">
                        {step}
                      </div>
                    ))}
                  </div>
                ) : null}
                {viewDiagnosis.summary.versionAwareness ? (
                  <div className="text-xs text-muted-foreground">
                    {viewDiagnosis.summary.versionAwareness}
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}
        </CardContent>
        </Card>
      ) : null}

      <div className="grid gap-3">
        {visibleSections.map((section) => (
          <Card key={section.key} className="gap-2 py-4">
            <details
              open={section.status !== "healthy" ? true : undefined}
              className="group"
            >
              <summary className="list-none cursor-pointer">
                <CardHeader className="pb-0">
                  <div className="flex items-center justify-between gap-3">
                    <div className="space-y-1">
                      <CardTitle className="text-sm">{section.title}</CardTitle>
                      <p className="text-sm text-muted-foreground">{section.summary}</p>
                    </div>
                    {shouldShowSectionStatusBadge(viewDiagnosis.summary.status, section.status) ? (
                      <div className="flex items-center gap-2">
                        <Badge
                          variant={section.status === "broken" ? "destructive" : "outline"}
                          className={statusBadgeClass(section.status)}
                        >
                          {translateStatus(section.status)}
                        </Badge>
                      </div>
                    ) : null}
                  </div>
                </CardHeader>
              </summary>
              <CardContent className="pt-3">
                <div className="grid gap-2">
                  {hasDocGuidance(section) ? (
                    <div className="rounded-md border border-border/60 bg-muted/20 p-3 text-sm">
                      <div className="space-y-2">
                        {section.rootCauseHypotheses?.map((hypothesis) => (
                          <div key={hypothesis.title}>
                            <div className="font-medium">{hypothesis.title}</div>
                            <div className="text-muted-foreground">{hypothesis.reason}</div>
                          </div>
                        ))}
                        {section.fixSteps?.length ? (
                          <div className="space-y-1">
                            {section.fixSteps.map((step, index) => (
                              <div key={`${step}-${index}`} className="text-muted-foreground">
                                {step}
                              </div>
                            ))}
                          </div>
                        ) : null}
                        {section.versionAwareness ? (
                          <div className="text-xs text-muted-foreground">
                            {section.versionAwareness}
                          </div>
                        ) : null}
                      </div>
                    </div>
                  ) : null}
                  {section.items.map((item) => (
                    <div
                      key={item.id}
                      className="rounded-md border border-border/50 bg-background/70 p-2"
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="min-w-0">
                          <div className="text-sm">{item.label}</div>
                          {item.detail ? (
                            <div className="mt-1 text-xs text-muted-foreground">
                              {item.detail}
                            </div>
                          ) : null}
                        </div>
                        <div className="flex items-center gap-2">
                          {showRepairActions && item.autoFixable && item.issueId ? (
                            <Button
                              variant="outline"
                              size="sm"
                              className="h-7 px-2 text-[11px]"
                              onClick={() => onRepairIssue(item.issueId!)}
                              disabled={checkLoading || repairing}
                            >
                              {item.status === "warn" || item.status === "info"
                                ? t("doctor.optimize", { defaultValue: "Optimize" })
                                : t("doctor.fix", { defaultValue: "Fix" })}
                            </Button>
                          ) : null}
                          {shouldShowItemStatusBadge(section.status, item.status) ? (
                            <Badge
                              variant={itemBadgeVariant(item.status)}
                              className={cn("text-[10px]", statusBadgeClass(item.status))}
                            >
                              {translateStatus(item.status)}
                            </Badge>
                          ) : null}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </details>
          </Card>
        ))}
      </div>
    </div>
  );
}
