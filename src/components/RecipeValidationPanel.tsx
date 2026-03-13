import { useTranslation } from "react-i18next";

import type { RecipeSourceDiagnostic, RecipeSourceDiagnostics } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

function formatDiagnosticLabel(diagnostic: RecipeSourceDiagnostic): string {
  const parts = [diagnostic.recipeId, diagnostic.path].filter(Boolean);
  return parts.join(" · ");
}

export function RecipeValidationPanel({
  diagnostics,
  validating,
  errorMessage,
}: {
  diagnostics: RecipeSourceDiagnostics;
  validating: boolean;
  errorMessage?: string | null;
}) {
  const { t } = useTranslation();
  const errorCount = diagnostics.errors.length;
  const warningCount = diagnostics.warnings.length;

  return (
    <Card>
      <CardHeader className="space-y-3">
        <div className="flex items-center justify-between gap-2 flex-wrap">
          <CardTitle>{t("recipeStudio.validationTitle")}</CardTitle>
          <div className="flex items-center gap-2">
            <Badge variant={errorCount > 0 ? "destructive" : "secondary"}>
              {t("recipeStudio.validationErrors", { count: errorCount })}
            </Badge>
            <Badge variant="outline">
              {t("recipeStudio.validationWarnings", { count: warningCount })}
            </Badge>
          </div>
        </div>
        <p className="text-sm text-muted-foreground">
          {validating
            ? t("recipeStudio.validationPending")
            : errorMessage
              ? errorMessage
              : errorCount === 0
                ? t("recipeStudio.validationClean")
                : t("recipeStudio.validationNeedsAttention")}
        </p>
      </CardHeader>
      <CardContent className="space-y-3">
        {diagnostics.errors.map((diagnostic, index) => (
          <div key={`error-${index}`} className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2">
            <div className="text-xs font-medium uppercase tracking-wide text-destructive">
              {diagnostic.category}
            </div>
            {formatDiagnosticLabel(diagnostic) && (
              <div className="mt-1 text-xs text-muted-foreground">
                {formatDiagnosticLabel(diagnostic)}
              </div>
            )}
            <p className="mt-1 text-sm">{diagnostic.message}</p>
          </div>
        ))}
        {diagnostics.warnings.map((diagnostic, index) => (
          <div key={`warning-${index}`} className="rounded-xl border px-3 py-2">
            <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {diagnostic.category}
            </div>
            {formatDiagnosticLabel(diagnostic) && (
              <div className="mt-1 text-xs text-muted-foreground">
                {formatDiagnosticLabel(diagnostic)}
              </div>
            )}
            <p className="mt-1 text-sm">{diagnostic.message}</p>
          </div>
        ))}
        {errorCount === 0 && warningCount === 0 && !errorMessage && !validating && (
          <div className="rounded-xl border border-dashed px-3 py-4 text-sm text-muted-foreground">
            {t("recipeStudio.validationEmpty")}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
