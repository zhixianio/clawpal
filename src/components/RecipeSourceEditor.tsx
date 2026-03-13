import { useTranslation } from "react-i18next";

import type { RecipeEditorOrigin } from "@/lib/types";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

export function RecipeSourceEditor({
  value,
  readOnly,
  origin,
  onChange,
}: {
  value: string;
  readOnly: boolean;
  origin: RecipeEditorOrigin;
  onChange: (nextValue: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <section className="space-y-3">
      <div className="space-y-1">
        <Label htmlFor="recipe-source-editor">{t("recipeStudio.sourceTitle")}</Label>
        <p className="text-sm text-muted-foreground">
          {readOnly
            ? t("recipeStudio.sourceReadOnlyHint")
            : t("recipeStudio.sourceHint")}
        </p>
        {origin === "external" && !readOnly && (
          <p className="text-xs text-muted-foreground">
            {t("recipeStudio.externalDraftHint")}
          </p>
        )}
      </div>
      <Textarea
        id="recipe-source-editor"
        aria-label={t("recipeStudio.sourceTitle")}
        value={value}
        readOnly={readOnly}
        onChange={(event) => onChange(event.target.value)}
        className="min-h-[28rem] font-mono text-xs leading-5"
      />
    </section>
  );
}
