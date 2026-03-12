import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { RecipeCard } from "../components/RecipeCard";
import type {
  Recipe,
  RecipeEditorOrigin,
  RecipeRuntimeInstance,
  RecipeRuntimeRun,
  RecipeStudioDraft,
  RecipeWorkspaceEntry,
} from "../lib/types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AsyncActionButton } from "@/components/ui/AsyncActionButton";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useApi } from "@/lib/use-api";
import { formatTime } from "@/lib/utils";

function displayRunStatus(status: string): string {
  return status.replace(/_/g, " ");
}

function formatRunSourceTrace(run: RecipeRuntimeRun): string | null {
  const parts = [run.sourceOrigin, run.sourceDigest, run.workspacePath]
    .filter((value): value is string => !!value && value.trim().length > 0);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function Recipes({
  onCook,
  onOpenStudio,
  onOpenRuntimeDashboard,
  initialRecipes,
  initialInstances,
  initialRuns,
}: {
  onCook: (id: string, source?: string) => void;
  onOpenStudio?: (draft: RecipeStudioDraft) => void;
  onOpenRuntimeDashboard?: () => void;
  initialRecipes?: Recipe[];
  initialInstances?: RecipeRuntimeInstance[];
  initialRuns?: RecipeRuntimeRun[];
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [recipes, setRecipes] = useState<Recipe[]>(() => initialRecipes ?? []);
  const [instances, setInstances] = useState<RecipeRuntimeInstance[]>(() => initialInstances ?? []);
  const [runs, setRuns] = useState<RecipeRuntimeRun[]>(() => initialRuns ?? []);
  const [source, setSource] = useState("");
  const [loadedSource, setLoadedSource] = useState<string | undefined>(undefined);
  const [workspaceEntries, setWorkspaceEntries] = useState<RecipeWorkspaceEntry[]>([]);
  const [sourcePreview, setSourcePreview] = useState<string | null>(null);
  const [sourcePreviewName, setSourcePreviewName] = useState<string>("");
  const [copiedSource, setCopiedSource] = useState(false);

  const load = async (nextSource: string) => {
    const value = nextSource.trim();
    try {
      const [nextRecipes, nextInstances, nextRuns, nextWorkspaceEntries] = await Promise.all([
        ua.listRecipes(value || undefined),
        ua.listRecipeInstances(),
        ua.listRecipeRuns(),
        ua.listRecipeWorkspaceEntries(),
      ]);
      setLoadedSource(value || undefined);
      setRecipes(nextRecipes);
      setInstances(nextInstances);
      setRuns(nextRuns);
      setWorkspaceEntries(nextWorkspaceEntries);
    } catch (e) {
      console.error("Failed to load recipes:", e);
    }
  };

  useEffect(() => {
    if (initialRecipes || initialInstances || initialRuns) {
      return;
    }
    void load("");
  }, [initialRecipes, initialInstances, initialRuns]);

  const latestRun = useMemo(
    () => [...runs].sort((left, right) => right.startedAt.localeCompare(left.startedAt))[0],
    [runs],
  );

  const latestRunByRecipe = useMemo(() => {
    const result = new Map<string, RecipeRuntimeRun>();
    const sorted = [...runs].sort((left, right) => right.startedAt.localeCompare(left.startedAt));
    for (const run of sorted) {
      if (!result.has(run.recipeId)) {
        result.set(run.recipeId, run);
      }
    }
    return result;
  }, [runs]);

  const instanceCountByRecipe = useMemo(() => {
    const counts = new Map<string, number>();
    for (const instance of instances) {
      counts.set(instance.recipeId, (counts.get(instance.recipeId) ?? 0) + 1);
    }
    return counts;
  }, [instances]);

  const handleViewSource = async (recipe: Recipe) => {
    try {
      const exported = await ua.exportRecipeSource(recipe.id, loadedSource);
      setSourcePreviewName(recipe.name);
      setSourcePreview(exported);
      setCopiedSource(false);
    } catch (error) {
      console.error("Failed to export recipe source:", error);
    }
  };

  const handleOpenStudio = async (recipe: Recipe, origin: RecipeEditorOrigin) => {
    if (!onOpenStudio) {
      return;
    }
    try {
      const exported = await ua.exportRecipeSource(recipe.id, loadedSource);
      onOpenStudio({
        recipeId: recipe.id,
        recipeName: recipe.name,
        source: exported,
        origin,
      });
    } catch (error) {
      console.error("Failed to open recipe studio:", error);
    }
  };

  const handleOpenWorkspaceEntry = async (entry: RecipeWorkspaceEntry) => {
    if (!onOpenStudio) {
      return;
    }
    try {
      const sourceText = await ua.readRecipeWorkspaceSource(entry.slug);
      onOpenStudio({
        recipeId: entry.slug,
        recipeName: entry.slug,
        source: sourceText,
        origin: "workspace",
        workspaceSlug: entry.slug,
      });
    } catch (error) {
      console.error("Failed to open workspace recipe:", error);
    }
  };

  const handleCopySource = async () => {
    if (!sourcePreview) return;
    const writer = navigator?.clipboard?.writeText;
    if (typeof writer !== "function") {
      return;
    }
    await writer.call(navigator.clipboard, sourcePreview);
    setCopiedSource(true);
    window.setTimeout(() => setCopiedSource(false), 1500);
  };

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('recipes.title')}</h2>
      <div className="mb-2 flex items-center gap-2">
        <Label>{t('recipes.sourceLabel')}</Label>
        <Input
          value={source}
          onChange={(event) => setSource(event.target.value)}
          placeholder="/path/recipes.json or https://example.com/recipes.json"
          className="w-[380px]"
        />
        <AsyncActionButton className="ml-2" onClick={() => load(source)} loadingText={t('recipes.loading')}>
          {t('recipes.load')}
        </AsyncActionButton>
      </div>
      <p className="text-sm text-muted-foreground mt-0">
        {t('recipes.loadedFrom', { source: loadedSource || t('recipes.builtinSource') })}
      </p>
      <Card className="mt-4 mb-4">
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="space-y-1">
              <div className="text-sm font-medium">{t("recipes.workspaceTitle")}</div>
              <p className="text-sm text-muted-foreground">
                {workspaceEntries.length > 0
                  ? t("recipes.workspaceDescription")
                  : t("recipes.workspaceEmpty")}
              </p>
            </div>
            <Badge variant="outline">{workspaceEntries.length}</Badge>
          </div>
          {workspaceEntries.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {workspaceEntries.map((entry) => (
                <Button
                  key={entry.slug}
                  variant="outline"
                  size="sm"
                  onClick={() => void handleOpenWorkspaceEntry(entry)}
                >
                  {t("recipes.workspaceOpen", { slug: entry.slug })}
                </Button>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
      <Card className="mt-4 mb-4">
        <CardContent className="space-y-4">
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div className="space-y-1">
              <div className="text-sm font-medium">{t("recipes.runtimeTitle")}</div>
              <p className="text-sm text-muted-foreground">
                {latestRun
                  ? t("recipes.runtimeDescription")
                  : t("recipes.runtimeEmpty")}
              </p>
            </div>
            {onOpenRuntimeDashboard && (
              <Button variant="outline" size="sm" onClick={onOpenRuntimeDashboard}>
                {t("recipes.runtimeOpenDashboard")}
              </Button>
            )}
          </div>

          <div className="grid gap-3 sm:grid-cols-3">
            <div className="rounded-xl border bg-muted/20 px-3 py-2">
              <div className="text-xs text-muted-foreground">{t("recipes.runtimeInstances")}</div>
              <div className="text-lg font-semibold">{instances.length}</div>
            </div>
            <div className="rounded-xl border bg-muted/20 px-3 py-2">
              <div className="text-xs text-muted-foreground">{t("recipes.runtimeRuns")}</div>
              <div className="text-lg font-semibold">{runs.length}</div>
            </div>
            <div className="rounded-xl border bg-muted/20 px-3 py-2">
              <div className="text-xs text-muted-foreground">{t("recipes.runtimeLastRun")}</div>
              <div className="text-sm font-medium">
                {latestRun ? formatTime(latestRun.startedAt) : t("recipes.runtimeNoRuns")}
              </div>
            </div>
          </div>

          {latestRun && (
            <div className="rounded-xl border bg-background px-3 py-3">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-medium">{t("recipes.runtimeRecentRun")}</span>
                <Badge variant="outline">{displayRunStatus(latestRun.status)}</Badge>
                <span className="text-xs text-muted-foreground">
                  {latestRun.recipeId} · {latestRun.runner}
                </span>
              </div>
              <p className="text-sm mt-2">{latestRun.summary}</p>
              {formatRunSourceTrace(latestRun) && (
                <p className="text-xs text-muted-foreground mt-1">
                  {t("recipes.runtimeSourceTrace")}: {formatRunSourceTrace(latestRun)}
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Card>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
        {recipes.map((recipe) => (
          <div key={recipe.id} className="space-y-2">
            <RecipeCard
              recipe={recipe}
              onCook={() => onCook(recipe.id, loadedSource)}
              onViewSource={() => void handleViewSource(recipe)}
              onEditSource={
                loadedSource
                  ? () => void handleOpenStudio(recipe, "external")
                  : undefined
              }
              onForkToWorkspace={
                !loadedSource
                  ? () => void handleOpenStudio(recipe, "workspace")
                  : undefined
              }
            />
            {(latestRunByRecipe.has(recipe.id) || instanceCountByRecipe.has(recipe.id)) && (
              <div className="rounded-xl border bg-muted/20 px-3 py-2 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{t("recipes.runtimeRecentRun")}</span>
                  <span className="text-muted-foreground">
                    {t("recipes.runtimeInstancesForRecipe", {
                      count: instanceCountByRecipe.get(recipe.id) ?? 0,
                    })}
                  </span>
                </div>
                {latestRunByRecipe.get(recipe.id) ? (
                  <>
                    <div className="mt-1 text-sm">
                      {latestRunByRecipe.get(recipe.id)?.summary}
                    </div>
                    <div className="mt-1 text-muted-foreground">
                      {displayRunStatus(latestRunByRecipe.get(recipe.id)?.status ?? "")}
                      {" · "}
                      {formatTime(latestRunByRecipe.get(recipe.id)?.startedAt ?? "")}
                    </div>
                    {formatRunSourceTrace(latestRunByRecipe.get(recipe.id)!) && (
                      <div className="mt-1 text-muted-foreground">
                        {t("recipes.runtimeSourceTrace")}: {formatRunSourceTrace(latestRunByRecipe.get(recipe.id)!)}
                      </div>
                    )}
                  </>
                ) : (
                  <div className="mt-1 text-muted-foreground">{t("recipes.runtimeNoRuns")}</div>
                )}
              </div>
            )}
          </div>
        ))}
      </div>
      <Dialog
        open={!!sourcePreview}
        onOpenChange={(open) => {
          if (!open) {
            setSourcePreview(null);
            setCopiedSource(false);
          }
        }}
      >
        <DialogContent className="max-w-4xl max-h-[80vh] flex flex-col">
          <DialogHeader>
            <DialogTitle>{t("recipes.sourceDialogTitle", { name: sourcePreviewName })}</DialogTitle>
          </DialogHeader>
          <div className="flex items-center justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => void handleCopySource()}>
              {copiedSource ? t("recipes.sourceCopied") : t("recipes.copySource")}
            </Button>
          </div>
          <pre className="mt-2 flex-1 overflow-auto rounded-xl border bg-muted/20 p-4 text-xs leading-5">
            <code>{sourcePreview}</code>
          </pre>
        </DialogContent>
      </Dialog>
    </section>
  );
}
