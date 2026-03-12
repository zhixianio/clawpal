import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { RecipeSaveDialog } from "@/components/RecipeSaveDialog";
import { RecipeSourceEditor } from "@/components/RecipeSourceEditor";
import { RecipeValidationPanel } from "@/components/RecipeValidationPanel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { RecipeEditorOrigin, RecipeSourceDiagnostics } from "@/lib/types";
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

export function RecipeStudio({
  recipeId,
  recipeName,
  initialSource,
  origin,
  workspaceSlug,
  onBack,
}: {
  recipeId: string;
  recipeName: string;
  initialSource: string;
  origin: RecipeEditorOrigin;
  workspaceSlug?: string;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [source, setSource] = useState(initialSource);
  const [baselineSource, setBaselineSource] = useState(initialSource);
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

  useEffect(() => {
    setSource(initialSource);
    setBaselineSource(initialSource);
    setCurrentRecipeId(recipeId);
    setCurrentRecipeName(recipeName);
    setCurrentOrigin(origin);
    setCurrentWorkspaceSlug(workspaceSlug ?? null);
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

  const readOnly = currentOrigin === "builtin";
  const dirty = source !== baselineSource;
  const summaryBadgeKey = useMemo(() => describeDirtyState(dirty), [dirty]);
  const nextSuggestedSlug = useMemo(
    () => currentWorkspaceSlug ?? suggestSlug(source, currentRecipeId),
    [currentRecipeId, currentWorkspaceSlug, source],
  );

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
            <div className="text-sm font-medium">{t("recipeStudio.sourceSummaryTitle")}</div>
            <p className="text-sm text-muted-foreground">
              {t("recipeStudio.sourceSummaryBody")}
            </p>
          </div>
          <div className="flex items-center gap-2 flex-wrap">
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
                >
                  {t("recipeStudio.saveAs")}
                </Button>
                <Button onClick={() => void handleSave()}>
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
            <Button variant="outline" onClick={handleBack}>
              {t("recipeStudio.back")}
            </Button>
          </div>
        </CardContent>
      </Card>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(20rem,0.85fr)]">
        <RecipeSourceEditor
          value={source}
          readOnly={readOnly}
          origin={currentOrigin}
          onChange={setSource}
        />
        <RecipeValidationPanel
          diagnostics={diagnostics}
          validating={validating}
          errorMessage={validationError}
        />
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
