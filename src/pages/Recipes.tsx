import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { FolderOpenIcon, PencilIcon, Trash2Icon } from "lucide-react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { RecipeCard } from "../components/RecipeCard";
import type {
  Recipe,
  RecipeEditorOrigin,
  RecipeSourceImportResult,
  RecipeRuntimeInstance,
  RecipeRuntimeRun,
  RecipeStudioDraft,
  RecipeWorkspaceEntry,
} from "../lib/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { firstDroppedRecipeSource } from "@/lib/recipe-source-input";
import { useApi } from "@/lib/use-api";
import { cn, formatTime } from "@/lib/utils";

function displayRunStatus(status: string): string {
  return status.replace(/_/g, " ");
}

function formatRunSourceTrace(run: RecipeRuntimeRun): string | null {
  const parts = [run.sourceOrigin, run.sourceDigest, run.workspacePath]
    .filter((value): value is string => !!value && value.trim().length > 0);
  return parts.length > 0 ? parts.join(" · ") : null;
}

type WorkspaceDraftPreview = {
  sourceText?: string;
  recipe: Recipe | null;
};

type PendingRecipeImportConflicts = {
  source: string;
  result: RecipeSourceImportResult;
};

function humanizeWorkspaceSlug(slug: string): string {
  return slug
    .split(/[-_]+/)
    .filter((part) => part.trim().length > 0)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function getWorkspaceSourceBadgeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  entry: RecipeWorkspaceEntry,
): string | null {
  switch (entry.sourceKind) {
    case "bundled":
      return t("recipes.workspaceSourceBundled");
    case "localImport":
      return t("recipes.workspaceSourceLocal");
    case "remoteUrl":
      return t("recipes.workspaceSourceUrl");
    default:
      return null;
  }
}

function getWorkspaceStateBadgeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  entry: RecipeWorkspaceEntry,
): string | null {
  switch (entry.bundledState) {
    case "upToDate":
      return t("recipes.workspaceStateUpToDate");
    case "updateAvailable":
      return t("recipes.workspaceStateUpdateAvailable");
    case "localModified":
      return t("recipes.workspaceStateModified");
    case "conflictedUpdate":
      return t("recipes.workspaceStateConflict");
    default:
      return null;
  }
}

function getWorkspaceStateBadgeVariant(entry: RecipeWorkspaceEntry): "outline" | "secondary" {
  return entry.bundledState === "updateAvailable" ? "secondary" : "outline";
}

function getWorkspaceRiskBadgeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  entry: RecipeWorkspaceEntry,
): string {
  switch (entry.riskLevel) {
    case "high":
      return t("recipes.workspaceRiskHigh");
    case "medium":
      return t("recipes.workspaceRiskMedium");
    default:
      return t("recipes.workspaceRiskLow");
  }
}

function CookIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" className="size-3.5 fill-current">
      <path d="M5 3.5v9l7-4.5-7-4.5Z" />
    </svg>
  );
}

export function Recipes({
  onCook,
  onOpenStudio,
  onOpenRuntimeDashboard,
  initialRecipes,
  initialInstances,
  initialRuns,
  initialWorkspaceEntries,
}: {
  onCook: (
    id: string,
    options?: {
      source?: string;
      sourceText?: string;
      sourceOrigin?: "saved" | "draft";
      workspaceSlug?: string;
    },
  ) => void;
  onOpenStudio?: (draft: RecipeStudioDraft) => void;
  onOpenRuntimeDashboard?: () => void;
  initialRecipes?: Recipe[];
  initialInstances?: RecipeRuntimeInstance[];
  initialRuns?: RecipeRuntimeRun[];
  initialWorkspaceEntries?: RecipeWorkspaceEntry[];
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [recipes, setRecipes] = useState<Recipe[]>(() => initialRecipes ?? []);
  const [instances, setInstances] = useState<RecipeRuntimeInstance[]>(() => initialInstances ?? []);
  const [runs, setRuns] = useState<RecipeRuntimeRun[]>(() => initialRuns ?? []);
  const [source, setSource] = useState("");
  const [workspaceEntries, setWorkspaceEntries] = useState<RecipeWorkspaceEntry[]>(
    () => initialWorkspaceEntries ?? [],
  );
  const [workspaceDrafts, setWorkspaceDrafts] = useState<Record<string, WorkspaceDraftPreview>>(
    {},
  );
  const [sourcePreview, setSourcePreview] = useState<string | null>(null);
  const [sourcePreviewName, setSourcePreviewName] = useState<string>("");
  const [copiedSource, setCopiedSource] = useState(false);
  const [importNotice, setImportNotice] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [pickingDirectory, setPickingDirectory] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const [pendingImportConflicts, setPendingImportConflicts] =
    useState<PendingRecipeImportConflicts | null>(null);
  const sourceDropTargetRef = useRef<HTMLDivElement | null>(null);

  const hydrateWorkspaceDrafts = async (
    entries: RecipeWorkspaceEntry[],
  ): Promise<Record<string, WorkspaceDraftPreview>> => {
    const previews = await Promise.all(
      entries.map(async (entry) => {
        try {
          const sourceText = await ua.readRecipeWorkspaceSource(entry.slug);
          const workspaceRecipes = await ua.listRecipesFromSourceText(sourceText);
          return [
            entry.slug,
            {
              sourceText,
              recipe: workspaceRecipes[0] ?? null,
            },
          ] as const;
        } catch (error) {
          console.error("Failed to hydrate workspace recipe preview:", error);
          return [
            entry.slug,
            {
              recipe: null,
            },
          ] as const;
        }
      }),
    );

    return Object.fromEntries(previews);
  };

  const refreshPage = async () => {
    try {
      const [nextRecipes, nextInstances, nextRuns, nextWorkspaceEntries] = await Promise.all([
        ua.listRecipes(),
        ua.listRecipeInstances(),
        ua.listRecipeRuns(),
        ua.listRecipeWorkspaceEntries(),
      ]);
      const nextWorkspaceDrafts = await hydrateWorkspaceDrafts(nextWorkspaceEntries);
      setRecipes(nextRecipes);
      setInstances(nextInstances);
      setRuns(nextRuns);
      setWorkspaceEntries(nextWorkspaceEntries);
      setWorkspaceDrafts(nextWorkspaceDrafts);
    } catch (e) {
      console.error("Failed to load recipes:", e);
      setImportNotice(
        t("recipes.loadFailed", {
          error: e instanceof Error ? e.message : String(e),
        }),
      );
    }
  };

  const buildImportNotice = (result: RecipeSourceImportResult): string => {
    const parts = [
      t("recipes.importSummary", {
        imported: result.imported.length,
        skipped: result.skipped.length,
      }),
    ];
    if (result.warnings.length > 0) {
      parts.push(
        t("recipes.importWarnings", {
          count: result.warnings.length,
        }),
      );
    }
    return parts.join(" ");
  };

  const runImport = async (requestedSource: string, overwriteExisting = false) => {
    const value = requestedSource.trim();
    if (!value) {
      setImportNotice(t("recipes.importRequiresPath"));
      return;
    }
    setImporting(true);
    try {
      const result = await ua.importRecipeSource(value, overwriteExisting);
      if (!overwriteExisting && result.conflicts.length > 0) {
        setPendingImportConflicts({
          source: value,
          result,
        });
        setImportNotice(
          t("recipes.importConflictSummary", {
            count: result.conflicts.length,
          }),
        );
        return;
      }

      setPendingImportConflicts(null);
      await refreshPage();
      setImportNotice(buildImportNotice(result));
    } catch (error) {
      console.error("Failed to import recipe source:", error);
      setImportNotice(
        t("recipes.importFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    } finally {
      setImporting(false);
    }
  };

  const handleImport = async () => runImport(source);

  const handlePickDirectory = async () => {
    setPickingDirectory(true);
    try {
      const selected = await ua.pickRecipeSourceDirectory();
      if (selected) {
        setSource(selected);
        setImportNotice(null);
      }
    } catch (error) {
      console.error("Failed to pick recipe source directory:", error);
      setImportNotice(
        t("recipes.pickDirectoryFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    } finally {
      setPickingDirectory(false);
    }
  };

  useEffect(() => {
    if (initialRecipes || initialInstances || initialRuns || initialWorkspaceEntries) {
      return;
    }
    void refreshPage();
  }, [initialRecipes, initialInstances, initialRuns, initialWorkspaceEntries]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const isDropInsideTarget = (position?: { x: number; y: number }) => {
      if (!position || !sourceDropTargetRef.current) {
        return false;
      }
      const rect = sourceDropTargetRef.current.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      const x = position.x / ratio;
      const y = position.y / ratio;
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    };

    void getCurrentWebviewWindow()
      .onDragDropEvent((event: any) => {
        const payload = event?.payload;
        if (!payload?.type) {
          return;
        }

        if (payload.type === "leave") {
          setDragActive(false);
          return;
        }

        if (payload.type === "enter" || payload.type === "over") {
          setDragActive(isDropInsideTarget(payload.position));
          return;
        }

        if (payload.type === "drop") {
          const isInside = isDropInsideTarget(payload.position);
          setDragActive(false);
          if (!isInside) {
            return;
          }
          const nextPath = firstDroppedRecipeSource(payload.paths ?? []);
          if (!nextPath) {
            return;
          }
          setSource(nextPath);
          setImportNotice(
            (payload.paths?.length ?? 0) > 1
              ? t("recipes.dropMultipleNotice")
              : t("recipes.dropReadyNotice"),
          );
        }
      })
      .then((cleanup) => {
        if (disposed) {
          cleanup();
          return;
        }
        unlisten = cleanup;
      })
      .catch((error) => {
        console.warn("Failed to bind recipe drag-drop listener:", error);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [t]);

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

  const workspaceDraftCards = useMemo(
    () =>
      workspaceEntries.map((entry) => {
        const preview = workspaceDrafts[entry.slug];
        const recipe = preview?.recipe ?? null;
        return {
          entry,
          displayName: recipe?.name?.trim() || humanizeWorkspaceSlug(entry.slug),
          description:
            recipe?.description?.trim() || t("recipes.workspaceDraftFallbackDescription"),
          tags: recipe?.tags?.length ? recipe.tags : [t("recipes.workspaceTag")],
          stepCount: recipe?.steps?.length ?? 0,
          difficulty: recipe?.difficulty ?? "normal",
        };
      }),
    [t, workspaceDrafts, workspaceEntries],
  );

  const handleViewSource = async (recipe: Recipe) => {
    try {
      const exported = await ua.exportRecipeSource(recipe.id);
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
      const exported = await ua.exportRecipeSource(recipe.id);
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
      const hydrated = workspaceDrafts[entry.slug];
      const sourceText = hydrated?.sourceText ?? await ua.readRecipeWorkspaceSource(entry.slug);
      const primaryRecipe = hydrated?.recipe ?? (await ua.listRecipesFromSourceText(sourceText))[0];
      onOpenStudio({
        recipeId: primaryRecipe?.id ?? entry.slug,
        recipeName: primaryRecipe?.name ?? entry.slug,
        source: sourceText,
        origin: "workspace",
        workspaceSlug: entry.slug,
      });
    } catch (error) {
      console.error("Failed to open workspace recipe:", error);
    }
  };

  const handleCookWorkspaceEntry = async (entry: RecipeWorkspaceEntry) => {
    try {
      const hydrated = workspaceDrafts[entry.slug];
      const sourceText = hydrated?.sourceText ?? await ua.readRecipeWorkspaceSource(entry.slug);
      const primaryRecipe = hydrated?.recipe ?? (await ua.listRecipesFromSourceText(sourceText))[0];
      onCook(primaryRecipe?.id ?? entry.slug, {
        sourceText,
        sourceOrigin: "saved",
        workspaceSlug: entry.slug,
      });
    } catch (error) {
      console.error("Failed to cook workspace recipe:", error);
    }
  };

  const handleDeleteWorkspaceEntry = async (entry: RecipeWorkspaceEntry) => {
    try {
      await ua.deleteRecipeWorkspaceSource(entry.slug);
      setWorkspaceEntries((current) => current.filter((item) => item.slug !== entry.slug));
      setWorkspaceDrafts((current) => {
        const next = { ...current };
        delete next[entry.slug];
        return next;
      });
      setImportNotice(t("recipes.workspaceDeleteSuccess", { slug: entry.slug }));
    } catch (error) {
      console.error("Failed to delete workspace recipe:", error);
      setImportNotice(
        t("recipes.workspaceDeleteFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    }
  };

  const handleUpgradeWorkspaceEntry = async (entry: RecipeWorkspaceEntry) => {
    try {
      await ua.upgradeBundledRecipeWorkspaceSource(entry.slug);
      await refreshPage();
      setImportNotice(
        t("recipes.workspaceUpdateSuccess", {
          slug: entry.recipeId ?? entry.slug,
        }),
      );
    } catch (error) {
      console.error("Failed to upgrade bundled recipe:", error);
      setImportNotice(
        t("recipes.workspaceUpdateFailed", {
          error: error instanceof Error ? error.message : String(error),
        }),
      );
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
      <h2 className="text-2xl font-bold mb-4">{t("recipes.title")}</h2>
      <div className="mb-2 flex items-center gap-2">
        <Label>{t("recipes.sourceLabel")}</Label>
        <div
          ref={sourceDropTargetRef}
          className={cn(
            "flex items-center gap-2 rounded-xl border p-1 transition-colors",
            dragActive && "border-primary bg-primary/5",
          )}
        >
          <Input
            value={source}
            onChange={(event) => setSource(event.target.value)}
            placeholder={t("recipes.sourcePlaceholder")}
            className="w-[380px] border-0 shadow-none focus-visible:ring-0"
          />
          <Button
            variant="ghost"
            size="icon-sm"
            type="button"
            title={t("recipes.pickDirectory")}
            aria-label={t("recipes.pickDirectory")}
            disabled={pickingDirectory}
            onClick={() => void handlePickDirectory()}
          >
            <FolderOpenIcon className="size-4" />
          </Button>
        </div>
        <Button type="button" onClick={() => void handleImport()} disabled={importing}>
          {importing ? t("recipes.importing") : t("recipes.import")}
        </Button>
      </div>
      <p className="text-sm text-muted-foreground mt-2">{t("recipes.sourceHelp")}</p>
      {importNotice && <p className="text-sm text-muted-foreground mt-2">{importNotice}</p>}
      <AlertDialog
        open={!!pendingImportConflicts}
        onOpenChange={(open) => {
          if (!open) {
            setPendingImportConflicts(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("recipes.importConflictTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("recipes.importConflictDescription", {
                count: pendingImportConflicts?.result.conflicts.length ?? 0,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-wrap gap-2">
            {pendingImportConflicts?.result.conflicts.map((conflict) => (
              <Badge key={conflict.slug} variant="outline">
                {conflict.recipeId}
              </Badge>
            ))}
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!pendingImportConflicts) {
                  return;
                }
                void runImport(pendingImportConflicts.source, true);
              }}
            >
              {t("recipes.importOverwrite")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
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
            <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
              {workspaceDraftCards.map((draft) => (
                <Card key={draft.entry.slug} className="group">
                  <CardHeader>
                    <CardTitle>{draft.displayName}</CardTitle>
                    <CardDescription>{draft.description}</CardDescription>
                  </CardHeader>
                  <CardContent>
                    <div className="flex flex-wrap gap-1.5 mb-3">
                      {getWorkspaceSourceBadgeLabel(t, draft.entry) && (
                        <Badge variant="outline">
                          {getWorkspaceSourceBadgeLabel(t, draft.entry)}
                        </Badge>
                      )}
                      {getWorkspaceStateBadgeLabel(t, draft.entry) && (
                        <Badge variant={getWorkspaceStateBadgeVariant(draft.entry)}>
                          {getWorkspaceStateBadgeLabel(t, draft.entry)}
                        </Badge>
                      )}
                      <Badge variant="outline">
                        {getWorkspaceRiskBadgeLabel(t, draft.entry)}
                      </Badge>
                      {draft.entry.approvalRequired && (
                        <Badge variant="secondary">
                          {t("recipes.workspaceApprovalRequired")}
                        </Badge>
                      )}
                      {draft.tags.map((tag) => (
                        <Badge
                          key={`${draft.entry.slug}-${tag}`}
                          variant="secondary"
                          className="bg-primary/8 text-primary/80 border-0"
                        >
                          {tag}
                        </Badge>
                      ))}
                    </div>
                    <p className="text-sm text-muted-foreground flex items-center gap-2">
                      <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full text-xs">
                        {t("recipeCard.steps", { count: draft.stepCount })}
                      </span>
                      <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full text-xs">
                        {t(`recipeCard.${draft.difficulty}`)}
                      </span>
                      {draft.entry.version && (
                        <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full text-xs">
                          {t("recipes.workspaceVersion", { version: draft.entry.version })}
                        </span>
                      )}
                    </p>
                    {draft.entry.bundledState === "conflictedUpdate" && (
                      <p className="mt-3 text-xs text-amber-700">
                        {t("recipes.workspaceConflictHint")}
                      </p>
                    )}
                  </CardContent>
                  <CardFooter>
                    <div className="flex items-center gap-1">
                      {draft.entry.bundledState === "updateAvailable" && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="px-2 text-xs"
                          onClick={() => void handleUpgradeWorkspaceEntry(draft.entry)}
                        >
                          {t("recipes.workspaceUpdate")}
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        title={t("recipes.workspaceCook")}
                        aria-label={t("recipes.workspaceCook")}
                        className="text-muted-foreground hover:text-foreground"
                        onClick={() => void handleCookWorkspaceEntry(draft.entry)}
                      >
                        <CookIcon />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        title={t("recipes.workspaceOpen", { slug: draft.displayName })}
                        aria-label={t("recipes.workspaceOpen", { slug: draft.displayName })}
                        className="text-muted-foreground hover:text-foreground"
                        onClick={() => void handleOpenWorkspaceEntry(draft.entry)}
                      >
                        <PencilIcon className="size-3.5" />
                      </Button>
                      <AlertDialog>
                        <AlertDialogTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title={t("recipes.workspaceDelete")}
                            aria-label={t("recipes.workspaceDelete")}
                            className="text-muted-foreground hover:text-destructive"
                          >
                            <Trash2Icon className="size-3.5" />
                          </Button>
                        </AlertDialogTrigger>
                        <AlertDialogContent>
                          <AlertDialogHeader>
                            <AlertDialogTitle>{t("recipes.workspaceDeleteTitle")}</AlertDialogTitle>
                            <AlertDialogDescription>
                              {t("recipes.workspaceDeleteDescription", {
                                name: draft.displayName,
                              })}
                            </AlertDialogDescription>
                          </AlertDialogHeader>
                          <AlertDialogFooter>
                            <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
                            <AlertDialogAction
                              variant="destructive"
                              onClick={() => void handleDeleteWorkspaceEntry(draft.entry)}
                            >
                              {t("recipes.workspaceDelete")}
                            </AlertDialogAction>
                          </AlertDialogFooter>
                        </AlertDialogContent>
                      </AlertDialog>
                    </div>
                  </CardFooter>
                </Card>
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
                {latestRun ? t("recipes.runtimeDescription") : t("recipes.runtimeEmpty")}
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
              onCook={() => onCook(recipe.id)}
              onViewSource={() => void handleViewSource(recipe)}
              onForkToWorkspace={() => void handleOpenStudio(recipe, "workspace")}
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
                    <div className="mt-1 text-sm">{latestRunByRecipe.get(recipe.id)?.summary}</div>
                    <div className="mt-1 text-muted-foreground">
                      {displayRunStatus(latestRunByRecipe.get(recipe.id)?.status ?? "")}
                      {" · "}
                      {formatTime(latestRunByRecipe.get(recipe.id)?.startedAt ?? "")}
                    </div>
                    {formatRunSourceTrace(latestRunByRecipe.get(recipe.id)!) && (
                      <div className="mt-1 text-muted-foreground">
                        {t("recipes.runtimeSourceTrace")}:{" "}
                        {formatRunSourceTrace(latestRunByRecipe.get(recipe.id)!)}
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
