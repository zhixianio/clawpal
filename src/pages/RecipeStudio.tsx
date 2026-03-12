import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { RecipeFormEditor } from "@/components/RecipeFormEditor";
import { RecipeSaveDialog } from "@/components/RecipeSaveDialog";
import { RecipeSampleParamsForm } from "@/components/RecipeSampleParamsForm";
import { RecipeSourceEditor } from "@/components/RecipeSourceEditor";
import { RecipeValidationPanel } from "@/components/RecipeValidationPanel";
import { RecipePlanPreview } from "@/components/RecipePlanPreview";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  fromRecipeEditorModel,
  parseRecipeSource,
  serializeRecipeEditorModel,
  toRecipeEditorModel,
} from "@/lib/recipe-editor-model";
import { getRecipeStudioActionState } from "@/lib/recipe-studio-actions";
import { getRecipeStudioProjectionDiff } from "@/lib/recipe-studio-projection-diff";
import type {
  Recipe,
  RecipeEditorModel,
  RecipeEditorOrigin,
  RecipePlan,
  RecipeSourceDiagnostics,
  RecipeStudioDraft,
} from "@/lib/types";
import { useApi } from "@/lib/use-api";

const EMPTY_DIAGNOSTICS: RecipeSourceDiagnostics = {
  errors: [],
  warnings: [],
};

const NEW_RECIPE_SOURCE = JSON.stringify(
  {
    id: "new-recipe",
    name: "New Recipe",
    description: "Describe what this recipe changes.",
    version: "1.0.0",
    tags: [],
    difficulty: "easy",
    params: [],
    steps: [],
    bundle: {
      apiVersion: "strategy.platform/v1",
      kind: "StrategyBundle",
      metadata: {},
      compatibility: {},
      inputs: [],
      capabilities: { allowed: [] },
      resources: { supportedKinds: [] },
      execution: { supportedKinds: ["attachment"] },
      runner: {},
      outputs: [],
    },
    executionSpecTemplate: {
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
  },
  null,
  2,
);

type SaveDialogMode = "save-as" | "fork" | null;
type RecipeStudioMode = "source" | "form";

function isRecipeShape(value: unknown): value is Recipe {
  return !!(
    value &&
    typeof value === "object" &&
    typeof (value as Recipe).id === "string" &&
    typeof (value as Recipe).name === "string" &&
    Array.isArray((value as Recipe).params) &&
    Array.isArray((value as Recipe).steps)
  );
}

function originLabelKey(origin: RecipeEditorOrigin): string {
  switch (origin) {
    case "workspace":
      return "recipeStudio.originWorkspace";
    case "external":
      return "recipeStudio.originExternal";
    default:
      return "recipeStudio.originBuiltin";
  }
}

function describeDirtyState(dirty: boolean): "recipeStudio.dirty" | "recipeStudio.saved" {
  return dirty ? "recipeStudio.dirty" : "recipeStudio.saved";
}

function projectionSectionLabelKey(section: string): string {
  switch (section) {
    case "documentShape":
      return "recipeStudio.projectionDiffSectionDocumentShape";
    default:
      return "recipeStudio.projectionDiffSectionGeneric";
  }
}

function tryParseRecipeIdentity(
  source: string,
  fallbackId: string,
  fallbackName: string,
): { recipeId: string; recipeName: string } {
  try {
    const parsed = JSON.parse(source) as
      | { id?: string; name?: string; recipes?: Array<{ id?: string; name?: string }> }
      | Array<{ id?: string; name?: string }>;
    const primary = Array.isArray(parsed)
      ? parsed[0]
      : Array.isArray(parsed?.recipes)
        ? parsed.recipes[0]
        : parsed;
    return {
      recipeId: primary?.id?.trim() || fallbackId,
      recipeName: primary?.name?.trim() || fallbackName,
    };
  } catch {
    return {
      recipeId: fallbackId,
      recipeName: fallbackName,
    };
  }
}

function tryParseRecipeFromSource(
  source: string,
  preferredRecipeId: string,
): Recipe | null {
  try {
    const parsed: unknown = JSON.parse(source);
    let candidates: unknown[] = [];
    if (Array.isArray(parsed)) {
      candidates = parsed;
    } else if (
      parsed &&
      typeof parsed === "object" &&
      Array.isArray((parsed as { recipes?: unknown[] }).recipes)
    ) {
      candidates = (parsed as { recipes: unknown[] }).recipes;
    } else {
      candidates = [parsed];
    }
    const validRecipes = candidates.filter(isRecipeShape);
    return validRecipes.find((item) => item.id === preferredRecipeId) ?? validRecipes[0] ?? null;
  } catch {
    return null;
  }
}

function suggestSlug(source: string, fallback: string): string {
  const { recipeId } = tryParseRecipeIdentity(source, fallback, fallback);
  const normalized = recipeId
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "recipe-draft";
}

function canConfirmDiscard(message: string): boolean {
  if (typeof window === "undefined" || typeof window.confirm !== "function") {
    return true;
  }
  return window.confirm(message);
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function tryBuildEditorModel(source: string): RecipeEditorModel | null {
  try {
    return toRecipeEditorModel(parseRecipeSource(source));
  } catch {
    return null;
  }
}

function syncDraftRecipeFromModel(model: RecipeEditorModel): Recipe | null {
  try {
    const nextDoc = fromRecipeEditorModel(model);
    return {
      id: nextDoc.id,
      name: nextDoc.name,
      description: nextDoc.description,
      version: nextDoc.version,
      tags: nextDoc.tags,
      difficulty: nextDoc.difficulty,
      params: nextDoc.params,
      steps: nextDoc.steps,
    };
  } catch {
    return null;
  }
}

export function RecipeStudio({
  recipeId,
  recipeName,
  initialSource,
  origin,
  workspaceSlug,
  onCookDraft,
  onBack,
}: {
  recipeId: string;
  recipeName: string;
  initialSource: string;
  origin: RecipeEditorOrigin;
  workspaceSlug?: string;
  onCookDraft?: (draft: RecipeStudioDraft) => void;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const initialDraftRecipe = useMemo(
    () => tryParseRecipeFromSource(initialSource, recipeId),
    [initialSource, recipeId],
  );
  const initialEditorModel = useMemo(
    () => tryBuildEditorModel(initialSource),
    [initialSource],
  );
  const [source, setSource] = useState(initialSource);
  const [baselineSource, setBaselineSource] = useState(initialSource);
  const [mode, setMode] = useState<RecipeStudioMode>("source");
  const [currentRecipeId, setCurrentRecipeId] = useState(recipeId);
  const [currentRecipeName, setCurrentRecipeName] = useState(recipeName);
  const [currentOrigin, setCurrentOrigin] = useState<RecipeEditorOrigin>(origin);
  const [currentWorkspaceSlug, setCurrentWorkspaceSlug] = useState<string | null>(
    workspaceSlug ?? null,
  );
  const [diagnostics, setDiagnostics] = useState<RecipeSourceDiagnostics>(EMPTY_DIAGNOSTICS);
  const [validating, setValidating] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [saveDialogMode, setSaveDialogMode] = useState<SaveDialogMode>(null);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [draftRecipe, setDraftRecipe] = useState<Recipe | null>(initialDraftRecipe);
  const [formModel, setFormModel] = useState<RecipeEditorModel | null>(initialEditorModel);
  const [formSyncError, setFormSyncError] = useState<string | null>(null);
  const [sampleParams, setSampleParams] = useState<Record<string, string>>(() => {
    if (!initialDraftRecipe) {
      return {};
    }
    const nextValues: Record<string, string> = {};
    for (const param of initialDraftRecipe.params) {
      nextValues[param.id] = param.defaultValue ?? (param.type === "boolean" ? "false" : "");
    }
    return nextValues;
  });
  const [planPreview, setPlanPreview] = useState<RecipePlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [planning, setPlanning] = useState(false);

  useEffect(() => {
    setSource(initialSource);
    setBaselineSource(initialSource);
    setMode("source");
    setCurrentRecipeId(recipeId);
    setCurrentRecipeName(recipeName);
    setCurrentOrigin(origin);
    setCurrentWorkspaceSlug(workspaceSlug ?? null);
    setDraftRecipe(tryParseRecipeFromSource(initialSource, recipeId));
    setFormModel(tryBuildEditorModel(initialSource));
    setFormSyncError(null);
    setSampleParams(() => {
      const parsedRecipe = tryParseRecipeFromSource(initialSource, recipeId);
      if (!parsedRecipe) {
        return {};
      }
      const nextValues: Record<string, string> = {};
      for (const param of parsedRecipe.params) {
        nextValues[param.id] = param.defaultValue ?? (param.type === "boolean" ? "false" : "");
      }
      return nextValues;
    });
    setPlanPreview(null);
    setPlanError(null);
  }, [initialSource, origin, recipeId, recipeName, workspaceSlug]);

  useEffect(() => {
    let cancelled = false;
    if (!source.trim()) {
      setDiagnostics(EMPTY_DIAGNOSTICS);
      setValidationError(null);
      setValidating(false);
      return () => {
        cancelled = true;
      };
    }

    setValidating(true);
    void ua.validateRecipeSourceText(source)
      .then((nextDiagnostics) => {
        if (cancelled) return;
        setDiagnostics(nextDiagnostics);
        setValidationError(null);
      })
      .catch((error) => {
        if (cancelled) return;
        setDiagnostics(EMPTY_DIAGNOSTICS);
        setValidationError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (!cancelled) {
          setValidating(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [source, ua]);

  useEffect(() => {
    if (!source.trim()) {
      setFormSyncError(null);
      return;
    }
    const nextModel = tryBuildEditorModel(source);
    if (nextModel) {
      setFormModel(nextModel);
      setFormSyncError(null);
      return;
    }
    setFormSyncError(t("recipeStudio.formSyncSourceError"));
  }, [source, t]);

  useEffect(() => {
    let cancelled = false;
    void ua.listRecipesFromSourceText(source)
      .then((recipes) => {
        if (cancelled) return;
        const preferred = recipes.find((item) => item.id === currentRecipeId) ?? recipes[0] ?? null;
        setDraftRecipe(preferred);
        setPlanPreview(null);
        setPlanError(null);
        if (!preferred) {
          setSampleParams({});
          return;
        }
        setSampleParams((current) => {
          const nextValues: Record<string, string> = {};
          for (const param of preferred.params) {
            nextValues[param.id] = current[param.id] ?? param.defaultValue ?? (param.type === "boolean" ? "false" : "");
          }
          return nextValues;
        });
      })
      .catch(() => {
        if (cancelled) return;
        setDraftRecipe(null);
        setSampleParams({});
        setPlanPreview(null);
        setPlanError(null);
      });

    return () => {
      cancelled = true;
    };
  }, [currentRecipeId, source, ua]);

  const readOnly = currentOrigin === "builtin";
  const dirty = source !== baselineSource;
  const summaryBadgeKey = useMemo(() => describeDirtyState(dirty), [dirty]);
  const nextSuggestedSlug = useMemo(
    () => currentWorkspaceSlug ?? suggestSlug(source, currentRecipeId),
    [currentRecipeId, currentWorkspaceSlug, source],
  );
  const modeSummaryTitle = mode === "form"
    ? t("recipeStudio.formSummaryTitle")
    : t("recipeStudio.sourceSummaryTitle");
  const modeSummaryBody = mode === "form"
    ? t("recipeStudio.formSummaryBody")
    : t("recipeStudio.sourceSummaryBody");
  const actionState = useMemo(
    () => getRecipeStudioActionState({
      source,
      validating,
      validationError,
      diagnostics,
      formSyncError,
      hasDraftRecipe: !!draftRecipe,
    }),
    [source, validating, validationError, diagnostics, formSyncError, draftRecipe],
  );
  const projectionDiff = useMemo(
    () => getRecipeStudioProjectionDiff(source),
    [source],
  );
  const saveBlockedReason = actionState.saveReasonKey ? t(actionState.saveReasonKey) : undefined;
  const previewBlockedReason = actionState.previewReasonKey ? t(actionState.previewReasonKey) : undefined;

  const completePersist = (slug: string) => {
    const identity = tryParseRecipeIdentity(source, currentRecipeId, currentRecipeName);
    setCurrentOrigin("workspace");
    setCurrentWorkspaceSlug(slug);
    setCurrentRecipeId(identity.recipeId);
    setCurrentRecipeName(identity.recipeName);
    setBaselineSource(source);
  };

  const persistDraft = async (slug: string) => {
    setSaving(true);
    try {
      const result = await ua.saveRecipeWorkspaceSource(slug, source);
      completePersist(result.slug);
      setSaveDialogMode(null);
      toast.success(t("recipeStudio.saveSuccess", { slug: result.slug }));
    } catch (error) {
      toast.error(
        t("recipeStudio.saveFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    } finally {
      setSaving(false);
    }
  };

  const handleSave = async () => {
    if (readOnly) {
      setSaveDialogMode("fork");
      return;
    }
    await persistDraft(nextSuggestedSlug);
  };

  const handleNewDraft = () => {
    if (dirty && !canConfirmDiscard(t("recipeStudio.unsavedResetConfirm"))) {
      return;
    }
    setCurrentRecipeId("new-recipe");
    setCurrentRecipeName("New Recipe");
    setCurrentOrigin("workspace");
    setCurrentWorkspaceSlug(null);
    setSource(NEW_RECIPE_SOURCE);
    setBaselineSource(NEW_RECIPE_SOURCE);
    setSaveDialogMode(null);
  };

  const handleBack = () => {
    if (dirty && !canConfirmDiscard(t("recipeStudio.unsavedLeaveConfirm"))) {
      return;
    }
    onBack();
  };

  const handleDelete = async () => {
    if (!currentWorkspaceSlug) {
      return;
    }
    if (!canConfirmDiscard(t("recipeStudio.deleteConfirm", { slug: currentWorkspaceSlug }))) {
      return;
    }
    setDeleting(true);
    try {
      await ua.deleteRecipeWorkspaceSource(currentWorkspaceSlug);
      toast.success(t("recipeStudio.deleteSuccess", { slug: currentWorkspaceSlug }));
      onBack();
    } catch (error) {
      toast.error(
        t("recipeStudio.deleteFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    } finally {
      setDeleting(false);
    }
  };

  const handlePreviewPlan = async () => {
    if (!draftRecipe) {
      return;
    }
    setPlanning(true);
    setPlanError(null);
    try {
      const nextPlan = await ua.planRecipeSource(draftRecipe.id, sampleParams, source);
      setPlanPreview(nextPlan);
    } catch (error) {
      setPlanPreview(null);
      setPlanError(error instanceof Error ? error.message : String(error));
    } finally {
      setPlanning(false);
    }
  };

  const handleFormChange = (nextModel: RecipeEditorModel) => {
    setFormModel(nextModel);
    const nextDraftRecipe = syncDraftRecipeFromModel(nextModel);
    if (nextDraftRecipe) {
      setDraftRecipe(nextDraftRecipe);
      setCurrentRecipeId(nextDraftRecipe.id);
      setCurrentRecipeName(nextDraftRecipe.name);
    }
    try {
      const nextSource = serializeRecipeEditorModel(nextModel);
      setSource(nextSource);
      setFormSyncError(null);
    } catch (error) {
      setFormSyncError(
        t("recipeStudio.formSyncFormError", { error: describeError(error) }),
      );
    }
  };

  return (
    <section className="space-y-4">
      <div className="flex items-start justify-between gap-3 flex-wrap">
        <div className="space-y-1">
          <h2 className="text-2xl font-bold">{t("recipeStudio.title")}</h2>
          <p className="text-sm text-muted-foreground">
            {currentRecipeName} · {currentRecipeId}
          </p>
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          <Badge variant="outline">{t(originLabelKey(currentOrigin))}</Badge>
          <Badge variant={readOnly ? "secondary" : "default"}>
            {t(readOnly ? "recipeStudio.readOnly" : "recipeStudio.editable")}
          </Badge>
          {!readOnly && (
            <Badge variant={dirty ? "default" : "outline"}>
              {t(summaryBadgeKey)}
            </Badge>
          )}
          {currentWorkspaceSlug && (
            <Badge variant="outline">{currentWorkspaceSlug}</Badge>
          )}
        </div>
      </div>

      <Card className="border-dashed bg-muted/10">
        <CardContent className="flex items-center justify-between gap-3 flex-wrap py-4">
          <div>
            <div className="text-sm font-medium">{modeSummaryTitle}</div>
            <p className="text-sm text-muted-foreground">
              {modeSummaryBody}
            </p>
          </div>
          <div className="flex items-center gap-2 flex-wrap">
            <Button
              variant={mode === "source" ? "default" : "outline"}
              onClick={() => setMode("source")}
            >
              {t("recipeStudio.modeSource")}
            </Button>
            <Button
              variant={mode === "form" ? "default" : "outline"}
              onClick={() => setMode("form")}
              disabled={!formModel}
            >
              {t("recipeStudio.modeForm")}
            </Button>
            <Button variant="outline" onClick={handleNewDraft}>
              {t("recipeStudio.new")}
            </Button>
            {readOnly ? (
              <Button onClick={() => setSaveDialogMode("fork")}>
                {t("recipeStudio.fork")}
              </Button>
            ) : (
              <>
                <Button
                  variant="outline"
                  onClick={() => setSaveDialogMode("save-as")}
                  disabled={actionState.saveDisabled}
                  title={saveBlockedReason}
                >
                  {t("recipeStudio.saveAs")}
                </Button>
                <Button
                  onClick={() => void handleSave()}
                  disabled={actionState.saveDisabled}
                  title={saveBlockedReason}
                >
                  {t("recipeStudio.save")}
                </Button>
              </>
            )}
            {currentWorkspaceSlug && (
              <Button
                variant="outline"
                onClick={() => void handleDelete()}
                disabled={deleting}
              >
                {t("recipeStudio.delete")}
              </Button>
            )}
            {onCookDraft && (
              <Button
                variant="outline"
                onClick={() => onCookDraft({
                  recipeId: draftRecipe?.id ?? currentRecipeId,
                  recipeName: draftRecipe?.name ?? currentRecipeName,
                  source,
                  origin: currentOrigin,
                  workspaceSlug: currentWorkspaceSlug ?? undefined,
                })}
                disabled={actionState.cookDisabled}
                title={previewBlockedReason}
              >
                {t("recipeStudio.cookDraft")}
              </Button>
            )}
            <Button variant="outline" onClick={handleBack}>
              {t("recipeStudio.back")}
            </Button>
          </div>
        </CardContent>
      </Card>

      {projectionDiff.hasDiff && (
        <Card className="border-amber-300/70 bg-amber-50/60 dark:border-amber-800 dark:bg-amber-950/20">
          <CardContent className="space-y-3 py-4">
            <div className="space-y-1">
              <div className="text-sm font-medium">{t("recipeStudio.projectionDiffTitle")}</div>
              <p className="text-sm text-muted-foreground">
                {mode === "source"
                  ? t("recipeStudio.projectionDiffSourceBody")
                  : t("recipeStudio.projectionDiffFormBody")}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              {projectionDiff.affectedSections.map((section) => {
                const labelKey = projectionSectionLabelKey(section);
                return (
                  <Badge key={section} variant="outline">
                    {labelKey === "recipeStudio.projectionDiffSectionGeneric"
                      ? t(labelKey, { section })
                      : t(labelKey)}
                  </Badge>
                );
              })}
            </div>
          </CardContent>
        </Card>
      )}

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(20rem,0.85fr)]">
        <div className="space-y-4">
          {mode === "source" ? (
            <RecipeSourceEditor
              value={source}
              readOnly={readOnly}
              origin={currentOrigin}
              onChange={setSource}
            />
          ) : formModel ? (
            <>
              <RecipeFormEditor
                model={formModel}
                readOnly={readOnly}
                onChange={handleFormChange}
              />
              {formSyncError && (
                <div className="rounded-xl border border-amber-300 bg-amber-50 px-3 py-3 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-100">
                  <div className="font-medium">{t("recipeStudio.formSyncErrorTitle")}</div>
                  <p>{formSyncError}</p>
                </div>
              )}
            </>
          ) : (
            <div className="rounded-xl border border-dashed px-4 py-6 text-sm text-muted-foreground">
              {t("recipeStudio.formUnavailable")}
            </div>
          )}
        </div>
        <div className="space-y-4">
          <RecipeValidationPanel
            diagnostics={diagnostics}
            validating={validating}
            errorMessage={validationError}
          />
          {draftRecipe && (
            <RecipeSampleParamsForm
              recipe={draftRecipe}
              values={sampleParams}
              onChange={(id, value) => {
                setSampleParams((current) => ({ ...current, [id]: value }));
              }}
              onPreviewPlan={() => void handlePreviewPlan()}
              planning={planning}
              previewDisabled={actionState.previewDisabled}
              disabledReason={previewBlockedReason}
            />
          )}
          {planError && (
            <div className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-3 text-sm text-destructive">
              <div className="font-medium">{t("recipeStudio.planErrorTitle")}</div>
              <div className="mt-1">{planError}</div>
            </div>
          )}
          {planPreview && (
            <div>
              <div className="mb-2 text-sm font-medium">{t("recipeStudio.planPreviewTitle")}</div>
              <RecipePlanPreview plan={planPreview} />
            </div>
          )}
        </div>
      </div>

      <RecipeSaveDialog
        open={saveDialogMode !== null}
        title={t(
          saveDialogMode === "fork"
            ? "recipeStudio.forkDialogTitle"
            : "recipeStudio.saveDialogTitle",
        )}
        confirmLabel={t(
          saveDialogMode === "fork"
            ? "recipeStudio.forkConfirm"
            : "recipeStudio.saveConfirm",
        )}
        initialSlug={nextSuggestedSlug}
        busy={saving}
        onOpenChange={(open) => {
          if (!open) {
            setSaveDialogMode(null);
          }
        }}
        onConfirm={(slug) => void persistDraft(slug)}
      />
    </section>
  );
}
