import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { hasGuidanceEmitted, useApi } from "@/lib/use-api";
import { formatBytes, formatTime } from "@/lib/utils";
import type { BackupInfo } from "@/lib/types";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AsyncActionButton } from "@/components/ui/AsyncActionButton";
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

export function BackupsPanel() {
  const { t } = useTranslation();
  const ua = useApi();
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [backupMessage, setBackupMessage] = useState("");
  const [deletingBackupName, setDeletingBackupName] = useState<string | null>(null);
  const [fadingOutBackupName, setFadingOutBackupName] = useState<string | null>(null);

  const refreshBackups = useCallback(() => {
    ua.listBackups()
      .then(setBackups)
      .catch((e) => console.error("Failed to load backups:", e));
  }, [ua]);

  useEffect(() => {
    setBackups(null);
    setBackupMessage("");
    setDeletingBackupName(null);
    setFadingOutBackupName(null);
    refreshBackups();
  }, [refreshBackups, ua.instanceId, ua.instanceToken, ua.isRemote, ua.isConnected]);

  return (
    <>
      <div className="flex items-center justify-between mb-4">
        <AsyncActionButton
          size="sm"
          variant="outline"
          loadingText={t("home.creating")}
          onClick={async () => {
            setBackupMessage("");
            try {
              const info = await ua.backupBeforeUpgrade();
              setBackupMessage(t("home.backupCreated", { name: info.name }));
              refreshBackups();
            } catch (e) {
              if (!hasGuidanceEmitted(e)) {
                setBackupMessage(t("home.backupFailed", { error: String(e) }));
              }
            }
          }}
        >
          {t("home.createBackup")}
        </AsyncActionButton>
      </div>
      {backupMessage && (
        <p className="text-sm text-muted-foreground mb-2">{backupMessage}</p>
      )}
      {backups === null ? (
        <div className="space-y-2">
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      ) : backups.length === 0 ? (
        <p className="text-muted-foreground text-sm">{t("doctor.noBackups")}</p>
      ) : (
        <div className="space-y-2">
          {backups.map((backup) => (
            <Card
              key={backup.name}
              className={`overflow-hidden transition-all duration-300 ease-out ${
                fadingOutBackupName === backup.name
                  ? "opacity-0 max-h-0"
                  : "opacity-100 max-h-40"
              }`}
            >
              <CardContent className="flex items-center justify-between">
                <div>
                  <div className="font-medium text-sm">{backup.name}</div>
                  <div className="text-xs text-muted-foreground">
                    {formatTime(backup.createdAt)} — {formatBytes(backup.sizeBytes)}
                  </div>
                  {ua.isRemote && backup.path && (
                    <div className="text-xs text-muted-foreground mt-0.5 font-mono">{backup.path}</div>
                  )}
                </div>
                <div className="flex gap-1.5">
                  {!ua.isRemote && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={deletingBackupName != null}
                      onClick={() => ua.openUrl(backup.path)}
                    >
                      {t("home.show")}
                    </Button>
                  )}
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button size="sm" variant="outline" disabled={deletingBackupName != null}>
                        {t("home.restore")}
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>{t("home.restoreTitle")}</AlertDialogTitle>
                        <AlertDialogDescription>
                          {t("home.restoreDescription", { name: backup.name })}
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
                        <AlertDialogAction
                          onClick={() => {
                            ua.restoreFromBackup(backup.name)
                              .then((msg) => setBackupMessage(msg))
                              .catch((e) => {
                                if (!hasGuidanceEmitted(e)) {
                                  setBackupMessage(t("home.restoreFailed", { error: String(e) }));
                                }
                              });
                          }}
                        >
                          {t("home.restore")}
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button
                        size="sm"
                        variant="destructive"
                        disabled={deletingBackupName != null || fadingOutBackupName === backup.name}
                      >
                        {t("home.delete")}
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>{t("home.deleteBackupTitle")}</AlertDialogTitle>
                        <AlertDialogDescription>
                          {t("home.deleteBackupDescription", { name: backup.name })}
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
                        <AlertDialogAction asChild>
                          <AsyncActionButton
                            variant="destructive"
                            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                            loadingText={t("home.deleting")}
                            disabled={deletingBackupName != null}
                            onClick={async () => {
                              setDeletingBackupName(backup.name);
                              try {
                                await ua.deleteBackup(backup.name);
                                setFadingOutBackupName(backup.name);
                                setBackupMessage(t("home.deletedBackup", { name: backup.name }));
                                setTimeout(() => {
                                  setBackups((prev) => prev?.filter((b) => b.name !== backup.name) ?? null);
                                  setFadingOutBackupName((prev) => (prev === backup.name ? null : prev));
                                  refreshBackups();
                                }, 350);
                              } catch (e) {
                                if (!hasGuidanceEmitted(e)) {
                                  setBackupMessage(t("home.deleteBackupFailed", { error: String(e) }));
                                }
                              } finally {
                                setDeletingBackupName(null);
                              }
                            }}
                          >
                            {t("home.delete")}
                          </AsyncActionButton>
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
