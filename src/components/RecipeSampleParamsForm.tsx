import { useTranslation } from "react-i18next";

import { ParamForm } from "@/components/ParamForm";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { Recipe } from "@/lib/types";

export function RecipeSampleParamsForm({
  recipe,
  values,
  onChange,
  onPreviewPlan,
  planning,
  previewDisabled = false,
  disabledReason,
}: {
  recipe: Recipe;
  values: Record<string, string>;
  onChange: (id: string, value: string) => void;
  onPreviewPlan: () => void;
  planning: boolean;
  previewDisabled?: boolean;
  disabledReason?: string | null;
}) {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader className="space-y-1">
        <CardTitle>{t("recipeStudio.sampleParamsTitle")}</CardTitle>
        <p className="text-sm text-muted-foreground">
          {t("recipeStudio.sampleParamsDescription")}
        </p>
        {disabledReason && (
          <p className="text-sm text-amber-700 dark:text-amber-300">
            {disabledReason}
          </p>
        )}
      </CardHeader>
      <CardContent>
        <ParamForm
          recipe={recipe}
          values={values}
          onChange={onChange}
          onSubmit={onPreviewPlan}
          submitLabel={planning ? t("recipeStudio.previewPlanPending") : t("recipeStudio.previewPlan")}
          submitDisabled={previewDisabled}
        />
      </CardContent>
    </Card>
  );
}
