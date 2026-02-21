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
import type { HistoryItem, PreviewResult } from "../lib/types";
import { formatTime } from "@/lib/utils";

export function History() {
  const { t } = useTranslation();
  const ua = useApi();
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [message, setMessage] = useState("");

  const refreshHistory = () => {
    return ua.listHistory()
      .then((resp) => setHistory(resp.items))
      .catch(() => setMessage(t('history.failedLoad')));
  };

  useEffect(() => {
    refreshHistory();
  }, [ua]);

  // Build a map from snapshot ID to its display info for rollback references
  const historyMap = new Map(
    history.map((h) => [h.id, h])
  );

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('history.title')}</h2>
      <div className="space-y-3">
        {history.map((item) => {
          const isRollback = item.source === "rollback";
          const rollbackTarget = item.rollbackOf ? historyMap.get(item.rollbackOf) : undefined;
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
                                const p = await ua.previewRollback(item.id);
                                const label = `Rollback to ${item.recipeId || formatTime(item.createdAt)}`;
                                await ua.queueCommand(label, ["__config_write__", p.configAfter]);
                                setMessage(t('history.rollbackQueued'));
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
