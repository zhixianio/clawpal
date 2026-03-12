import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/lib/use-api";
import { DiffViewer } from "../components/DiffViewer";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import type { HistoryItem, PreviewResult, RecipeRuntimeRun } from "../lib/types";
import { formatTime } from "@/lib/utils";

function formatResourceClaimLabel(run: RecipeRuntimeRun, index: number) {
  const claim = run.resourceClaims[index];
  return claim.id || claim.path || claim.target || claim.kind;
}

function formatRunSourceTrace(run: RecipeRuntimeRun): string | null {
  const parts = [run.sourceOrigin, run.sourceDigest, run.workspacePath]
    .filter((value): value is string => !!value && value.trim().length > 0);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function History({
  onOpenRuntimeDashboard,
  initialHistory = [],
  initialRuns = [],
}: {
  onOpenRuntimeDashboard?: () => void;
  initialHistory?: HistoryItem[];
  initialRuns?: RecipeRuntimeRun[];
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [history, setHistory] = useState<HistoryItem[]>(initialHistory);
  const [runtimeRuns, setRuntimeRuns] = useState<RecipeRuntimeRun[]>(initialRuns);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [message, setMessage] = useState("");

  const refreshHistory = () => {
    return Promise.all([
      ua.listHistory(),
      ua.listRecipeRuns().catch(() => [] as RecipeRuntimeRun[]),
    ])
      .then(([resp, runs]) => {
        setHistory(resp.items);
        setRuntimeRuns(runs);
      })
      .catch(() => setMessage(t('history.failedLoad')));
  };

  useEffect(() => {
    refreshHistory();
  }, [ua]);

  // Build a map from snapshot ID to its display info for rollback references
  const historyMap = new Map(
    history.map((h) => [h.id, h])
  );
  const runtimeRunMap = new Map(
    runtimeRuns.map((run) => [run.id, run])
  );
  const latestRun = runtimeRuns[0];

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('history.title')}</h2>
      <Card className="mb-4">
        <CardContent>
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div className="space-y-1">
              <div className="text-sm font-medium">{t("history.runtimeTitle")}</div>
              {latestRun ? (
                <>
                  <div className="flex items-center gap-2 flex-wrap">
                    <Badge variant="outline">{latestRun.status}</Badge>
                    <span className="text-sm">{latestRun.summary}</span>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {formatTime(latestRun.startedAt)} · {latestRun.instanceId} · {latestRun.runner}
                  </p>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">{t("history.runtimeEmpty")}</p>
              )}
            </div>
            {onOpenRuntimeDashboard && (
              <Button variant="outline" size="sm" onClick={onOpenRuntimeDashboard}>
                {t("history.runtimeOpenDashboard")}
              </Button>
            )}
          </div>
        </CardContent>
      </Card>
      <div className="space-y-3">
        {history.map((item) => {
          const isRollback = item.source === "rollback";
          const rollbackTarget = item.rollbackOf ? historyMap.get(item.rollbackOf) : undefined;
          const associatedRun = item.runId ? runtimeRunMap.get(item.runId) : undefined;
          const associatedArtifacts = associatedRun?.artifacts ?? item.artifacts ?? [];
          return (
            <Card key={item.id} className={isRollback ? "border-dashed opacity-75" : ""}>
              <CardContent>
                <div className="flex items-center gap-2 text-sm flex-wrap">
                  <span className="text-muted-foreground">{formatTime(item.createdAt)}</span>
                  {isRollback ? (
                    <>
                      <Badge variant="outline">{t('history.rollback')}</Badge>
                      <span className="text-muted-foreground">
                        {t('history.reverted', {
                          details: rollbackTarget
                            ? t('history.revertedRecipe', {
                                recipeId: rollbackTarget.recipeId || t('history.manual'),
                                time: formatTime(rollbackTarget.createdAt),
                              })
                            : item.recipeId || t('history.unknown'),
                        })}
                      </span>
                    </>
                  ) : (
                    <>
                      <Badge variant="secondary">{item.recipeId || t('history.manual')}</Badge>
                      <span className="text-muted-foreground">{item.source}</span>
                    </>
                  )}
                  {!item.canRollback && !isRollback && (
                    <Badge variant="outline" className="text-muted-foreground">{t('history.notRollbackable')}</Badge>
                  )}
                </div>
                {associatedRun && (
                  <div className="mt-3 space-y-1">
                    <div className="flex items-center gap-2 flex-wrap text-sm">
                      <Badge variant="outline">{associatedRun.status}</Badge>
                      <span>{associatedRun.summary}</span>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {t("history.runId")}: {associatedRun.id} · {associatedRun.runner} · {formatTime(associatedRun.startedAt)}
                    </p>
                    {associatedRun.resourceClaims.length > 0 && (
                      <p className="text-xs text-muted-foreground">
                        {t("history.runClaims")}: {associatedRun.resourceClaims.map((_, index) => formatResourceClaimLabel(associatedRun, index)).join(", ")}
                      </p>
                    )}
                    {formatRunSourceTrace(associatedRun) && (
                      <p className="text-xs text-muted-foreground">
                        {t("history.sourceTrace")}: {formatRunSourceTrace(associatedRun)}
                      </p>
                    )}
                  </div>
                )}
                {associatedArtifacts.length > 0 && (
                  <p className="mt-3 text-xs text-muted-foreground">
                    {t("history.runArtifacts")}: {associatedArtifacts.map((artifact) => artifact.label).join(", ")}
                  </p>
                )}
                {!isRollback && (
                  <div className="flex gap-2 mt-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={async () => {
                        try {
                          const p = await ua.previewRollback(item.id);
                          setPreview(p);
                        } catch (err) {
                          setMessage(String(err));
                        }
                      }}
                      disabled={!item.canRollback}
                    >
                      {t('history.preview')}
                    </Button>
                    <AlertDialog>
                      <AlertDialogTrigger asChild>
                        <Button
                          variant="destructive"
                          size="sm"
                          disabled={!item.canRollback}
                        >
                          {t('history.rollbackBtn')}
                        </Button>
                      </AlertDialogTrigger>
                      <AlertDialogContent>
                        <AlertDialogHeader>
                          <AlertDialogTitle>{t('history.rollbackConfirmTitle')}</AlertDialogTitle>
                          <AlertDialogDescription>
                            {t('history.rollbackConfirmDescription')}
                          </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                          <AlertDialogCancel>{t('config.cancel')}</AlertDialogCancel>
                          <AlertDialogAction
                            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                            onClick={async () => {
                              try {
                                await ua.rollback(item.id);
                                setMessage(t('history.rollbackCompleted'));
                                await refreshHistory();
                              } catch (err) {
                                setMessage(String(err));
                              }
                            }}
                          >
                            {t('history.rollbackBtn')}
                          </AlertDialogAction>
                        </AlertDialogFooter>
                      </AlertDialogContent>
                    </AlertDialog>
                  </div>
                )}
              </CardContent>
            </Card>
          );
        })}
      </div>
      <Button variant="outline" onClick={refreshHistory} className="mt-3">
        {t('history.refresh')}
      </Button>
      {message && (
        <p className="text-sm text-muted-foreground mt-2">{message}</p>
      )}

      {/* Preview Dialog */}
      <Dialog open={!!preview} onOpenChange={(open) => { if (!open) setPreview(null); }}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{t('history.rollbackPreview')}</DialogTitle>
          </DialogHeader>
          {preview && (
            <DiffViewer
              oldValue={preview.configBefore}
              newValue={preview.configAfter}
            />
          )}
        </DialogContent>
      </Dialog>
    </section>
  );
}
