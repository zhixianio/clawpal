import i18n from "@/i18n";
import type { RecipeParam } from "@/lib/types";

export function isParamVisible(
  param: RecipeParam,
  values: Record<string, string>,
): boolean {
  if (!param.dependsOn) return true;
  return values[param.dependsOn] === "true";
}

function validateField(param: RecipeParam, value: string): string | null {
  const trim = value.trim();
  if (param.required && trim.length === 0) {
    return i18n.t("paramForm.isRequired", { label: param.label });
  }
  if (
    param.type === "discord_guild" ||
    param.type === "discord_channel" ||
    param.type === "model_profile" ||
    param.type === "agent"
  ) {
    return null;
  }
  if (param.minLength != null && trim.length < param.minLength) {
    return i18n.t("paramForm.tooShort", { label: param.label });
  }
  if (param.maxLength != null && trim.length > param.maxLength) {
    return i18n.t("paramForm.tooLong", { label: param.label });
  }
  if (param.pattern && trim.length > 0) {
    try {
      if (!new RegExp(param.pattern).test(trim)) {
        return i18n.t("paramForm.invalidFormat", { label: param.label });
      }
    } catch {
      return i18n.t("paramForm.invalidRule", { label: param.label });
    }
  }
  return null;
}

export function validateVisibleParamValues(
  params: RecipeParam[],
  values: Record<string, string>,
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const param of params) {
    if (!isParamVisible(param, values)) continue;
    const error = validateField(param, values[param.id] || "");
    if (error) {
      next[param.id] = error;
    }
  }
  return next;
}

export function buildTouchedParamsOnSubmit(
  params: RecipeParam[],
  values: Record<string, string>,
): Record<string, boolean> {
  return Object.fromEntries(
    Object.keys(validateVisibleParamValues(params, values)).map((paramId) => [
      paramId,
      true,
    ]),
  );
}

export function findFirstInvalidVisibleParamId(
  params: RecipeParam[],
  values: Record<string, string>,
): string | null {
  const errors = validateVisibleParamValues(params, values);
  for (const param of params) {
    if (!isParamVisible(param, values)) continue;
    if (errors[param.id]) {
      return param.id;
    }
  }
  return null;
}
