import { useTranslation } from "react-i18next";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { RecipeEditorModel } from "@/lib/types";

function updateArrayItem<T>(items: T[], index: number, nextValue: T): T[] {
  return items.map((item, itemIndex) => (itemIndex === index ? nextValue : item));
}

export function RecipeFormEditor({
  model,
  readOnly,
  onChange,
}: {
  model: RecipeEditorModel;
  readOnly: boolean;
  onChange: (nextModel: RecipeEditorModel) => void;
}) {
  const { t } = useTranslation();

  return (
    <fieldset className="space-y-6" disabled={readOnly}>
      <section className="grid gap-3 md:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-id">{t("recipeStudio.form.id")}</Label>
          <Input
            id="recipe-form-id"
            value={model.id}
            onChange={(event) => onChange({ ...model, id: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-name">{t("recipeStudio.form.name")}</Label>
          <Input
            id="recipe-form-name"
            value={model.name}
            onChange={(event) => onChange({ ...model, name: event.target.value })}
          />
        </div>
        <div className="space-y-1.5 md:col-span-2">
          <Label htmlFor="recipe-form-description">{t("recipeStudio.form.description")}</Label>
          <Textarea
            id="recipe-form-description"
            value={model.description}
            onChange={(event) => onChange({ ...model, description: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-version">{t("recipeStudio.form.version")}</Label>
          <Input
            id="recipe-form-version"
            value={model.version}
            onChange={(event) => onChange({ ...model, version: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-tags">{t("recipeStudio.form.tags")}</Label>
          <Input
            id="recipe-form-tags"
            value={model.tagsText}
            onChange={(event) => onChange({ ...model, tagsText: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label>{t("recipeStudio.form.difficulty")}</Label>
          <Select
            value={model.difficulty}
            onValueChange={(value) => onChange({
              ...model,
              difficulty: value as RecipeEditorModel["difficulty"],
            })}
            disabled={readOnly}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="easy">{t("recipeStudio.form.difficultyEasy")}</SelectItem>
              <SelectItem value="normal">{t("recipeStudio.form.difficultyNormal")}</SelectItem>
              <SelectItem value="advanced">{t("recipeStudio.form.difficultyAdvanced")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label>{t("recipeStudio.form.executionKind")}</Label>
          <Select
            value={model.executionKind}
            onValueChange={(value) => onChange({
              ...model,
              executionKind: value as RecipeEditorModel["executionKind"],
            })}
            disabled={readOnly}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="attachment">attachment</SelectItem>
              <SelectItem value="job">job</SelectItem>
              <SelectItem value="service">service</SelectItem>
              <SelectItem value="schedule">schedule</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </section>

      <section className="space-y-3">
        <div className="text-sm font-medium">{t("recipeStudio.form.params")}</div>
        {model.params.length === 0 ? (
          <div className="rounded-xl border border-dashed px-3 py-4 text-sm text-muted-foreground">
            {t("recipeStudio.form.emptyParams")}
          </div>
        ) : (
          model.params.map((param, index) => (
            <div key={`${param.id}-${index}`} className="grid gap-2 rounded-xl border p-3 md:grid-cols-4">
              <Input
                aria-label={`${t("recipeStudio.form.paramId")} ${index + 1}`}
                value={param.id}
                onChange={(event) => onChange({
                  ...model,
                  params: updateArrayItem(model.params, index, { ...param, id: event.target.value }),
                })}
              />
              <Input
                aria-label={`${t("recipeStudio.form.paramLabel")} ${index + 1}`}
                value={param.label}
                onChange={(event) => onChange({
                  ...model,
                  params: updateArrayItem(model.params, index, { ...param, label: event.target.value }),
                })}
              />
              <Input
                aria-label={`${t("recipeStudio.form.paramType")} ${index + 1}`}
                value={param.type}
                onChange={(event) => onChange({
                  ...model,
                  params: updateArrayItem(model.params, index, { ...param, type: event.target.value as typeof param.type }),
                })}
              />
              <Input
                aria-label={`${t("recipeStudio.form.paramDefault")} ${index + 1}`}
                value={param.defaultValue ?? ""}
                onChange={(event) => onChange({
                  ...model,
                  params: updateArrayItem(model.params, index, { ...param, defaultValue: event.target.value }),
                })}
              />
            </div>
          ))
        )}
      </section>

      <section className="space-y-3">
        <div className="text-sm font-medium">{t("recipeStudio.form.steps")}</div>
        {model.steps.length === 0 ? (
          <div className="rounded-xl border border-dashed px-3 py-4 text-sm text-muted-foreground">
            {t("recipeStudio.form.emptySteps")}
          </div>
        ) : (
          model.steps.map((step, index) => (
            <div key={`${step.label}-${index}`} className="space-y-2 rounded-xl border p-3">
              <Input
                aria-label={`${t("recipeStudio.form.stepLabel")} ${index + 1}`}
                value={step.label}
                onChange={(event) => onChange({
                  ...model,
                  steps: updateArrayItem(model.steps, index, { ...step, label: event.target.value }),
                })}
              />
              <Input
                aria-label={`${t("recipeStudio.form.stepAction")} ${index + 1}`}
                value={step.action}
                onChange={(event) => onChange({
                  ...model,
                  steps: updateArrayItem(model.steps, index, { ...step, action: event.target.value }),
                })}
              />
              <Textarea
                aria-label={`${t("recipeStudio.form.stepArgs")} ${index + 1}`}
                value={JSON.stringify(step.args, null, 2)}
                onChange={(event) => {
                  try {
                    const nextArgs = JSON.parse(event.target.value);
                    onChange({
                      ...model,
                      steps: updateArrayItem(model.steps, index, { ...step, args: nextArgs }),
                    });
                  } catch {
                    onChange({
                      ...model,
                      steps: updateArrayItem(model.steps, index, { ...step, args: step.args }),
                    });
                  }
                }}
              />
            </div>
          ))
        )}
      </section>

      <section className="space-y-3">
        <div className="text-sm font-medium">{t("recipeStudio.form.actions")}</div>
        {model.actionRows.length === 0 ? (
          <div className="rounded-xl border border-dashed px-3 py-4 text-sm text-muted-foreground">
            {t("recipeStudio.form.emptyActions")}
          </div>
        ) : (
          model.actionRows.map((action, index) => (
            <div key={`${action.kind}-${index}`} className="space-y-2 rounded-xl border p-3">
              <Input
                aria-label={`${t("recipeStudio.form.actionKind")} ${index + 1}`}
                value={action.kind}
                onChange={(event) => onChange({
                  ...model,
                  actionRows: updateArrayItem(model.actionRows, index, { ...action, kind: event.target.value }),
                })}
              />
              <Input
                aria-label={`${t("recipeStudio.form.actionName")} ${index + 1}`}
                value={action.name}
                onChange={(event) => onChange({
                  ...model,
                  actionRows: updateArrayItem(model.actionRows, index, { ...action, name: event.target.value }),
                })}
              />
              <Textarea
                aria-label={`${t("recipeStudio.form.actionArgs")} ${index + 1}`}
                value={action.argsText}
                onChange={(event) => onChange({
                  ...model,
                  actionRows: updateArrayItem(model.actionRows, index, { ...action, argsText: event.target.value }),
                })}
              />
            </div>
          ))
        )}
      </section>

      <section className="grid gap-3 md:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-capabilities">{t("recipeStudio.form.capabilities")}</Label>
          <Textarea
            id="recipe-form-capabilities"
            value={model.bundleCapabilities.join("\n")}
            onChange={(event) => onChange({
              ...model,
              bundleCapabilities: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean),
            })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="recipe-form-resources">{t("recipeStudio.form.resources")}</Label>
          <Textarea
            id="recipe-form-resources"
            value={model.bundleResources.join("\n")}
            onChange={(event) => onChange({
              ...model,
              bundleResources: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean),
            })}
          />
        </div>
      </section>

      <p className="text-xs text-muted-foreground">
        {t("recipeStudio.form.syncHint")}
      </p>
    </fieldset>
  );
}
