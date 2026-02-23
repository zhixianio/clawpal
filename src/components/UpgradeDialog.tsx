import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";

type Step = "confirm" | "backup" | "upgrading" | "done";

/** Strip ANSI escape codes from terminal output */
function stripAnsi(str: string): string {
  return str.replace(/\x1b\[[0-9;]*m/g, "").replace(/\x1b\[[0-9;]*[A-Za-z]/g, "");
}

export function UpgradeDialog({
  open,
  onOpenChange,
  isRemote,
  instanceId,
  currentVersion,
  latestVersion,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  isRemote: boolean;
  instanceId: string;
  currentVersion: string;
  latestVersion: string;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("confirm");
  const [backupName, setBackupName] = useState("");
  const [output, setOutput] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [showLog, setShowLog] = useState(false);

  const reset = () => {
    setStep("confirm");
    setBackupName("");
    setOutput("");
    setError("");
    setLoading(false);
    setShowLog(false);
  };

  const handleClose = (open: boolean) => {
    if (!open && !loading) {
      reset();
      onOpenChange(false);
    }
  };

  const startUpgrade = async () => {
    setStep("backup");
    await runBackup();
  };

  const runBackup = async () => {
    setLoading(true);
    setError("");
    try {
      const info = isRemote
        ? await api.remoteBackupBeforeUpgrade(instanceId)
        : await api.backupBeforeUpgrade();
      setBackupName(info.name);
      setLoading(false);
      setStep("upgrading");
      await runUpgrade();
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  };

  const runUpgrade = async () => {
    setLoading(true);
    setError("");
    setOutput("");
    try {
      const result = isRemote
        ? await api.remoteRunOpenclawUpgrade(instanceId)
        : await api.runOpenclawUpgrade();
      setOutput(stripAnsi(result));
      setStep("done");
    } catch (e) {
      setOutput(stripAnsi(String(e)));
      setError(t('upgrade.upgradeFailed'));
      setShowLog(true);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {step === "confirm" && t('upgrade.title')}
            {step === "backup" && t('upgrade.backupTitle')}
            {step === "upgrading" && t('upgrade.upgradingTitle')}
            {step === "done" && t('upgrade.doneTitle')}
          </DialogTitle>
        </DialogHeader>

        {step === "confirm" && (
          <div className="space-y-3">
            <div className="flex items-center gap-2 text-sm">
              <span className="text-muted-foreground">{t('upgrade.current')}</span>
              <code className="font-medium">{currentVersion}</code>
              <span className="text-muted-foreground mx-1">&rarr;</span>
              <span className="text-muted-foreground">{t('upgrade.new')}</span>
              <code className="font-medium text-primary">{latestVersion}</code>
            </div>
            <p className="text-sm text-muted-foreground">
              {isRemote ? t('upgrade.confirmDescriptionRemote') : t('upgrade.confirmDescription')}
            </p>
          </div>
        )}

        {step === "backup" && (
          <div className="space-y-3">
            {loading && (
              <div className="flex items-center gap-2 text-sm">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span>{t('upgrade.creatingBackup')}</span>
              </div>
            )}
            {error && (
              <div className="text-sm text-destructive">{error}</div>
            )}
          </div>
        )}

        {step === "upgrading" && (
          <div className="space-y-3">
            {backupName && (
              <p className="text-sm text-muted-foreground">
                {t('upgrade.backupCreated')} <code>{backupName}</code>
              </p>
            )}
            {loading && (
              <div className="flex items-center gap-2 text-sm">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span>{t('upgrade.runningUpgrade')}</span>
              </div>
            )}
            {error && (
              <div className="text-sm text-destructive">{error}</div>
            )}
            {output && (
              <pre className="max-h-60 overflow-auto rounded-md bg-muted p-3 text-xs font-mono whitespace-pre-wrap">
                {output}
              </pre>
            )}
          </div>
        )}

        {step === "done" && (
          <div className="space-y-3">
            {backupName && (
              <p className="text-sm text-muted-foreground">
                {t('upgrade.backup')} <code>{backupName}</code>
              </p>
            )}
            <p className="text-sm font-medium text-green-600 dark:text-green-400">
              {t('upgrade.upgradeSuccess')}
            </p>
            {output && (
              <>
                <button
                  className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                  onClick={() => setShowLog(!showLog)}
                >
                  {showLog ? t('upgrade.hideDetails') : t('upgrade.showDetails')}
                </button>
                {showLog && (
                  <pre className="max-h-60 overflow-auto rounded-md bg-muted p-3 text-xs font-mono whitespace-pre-wrap">
                    {output}
                  </pre>
                )}
              </>
            )}
          </div>
        )}

        <DialogFooter>
          {step === "confirm" && (
            <>
              <Button variant="outline" onClick={() => handleClose(false)}>
                {t('upgrade.cancel')}
              </Button>
              <Button onClick={startUpgrade}>{t('upgrade.startUpgrade')}</Button>
            </>
          )}
          {step === "backup" && error && (
            <>
              <Button variant="outline" onClick={() => handleClose(false)}>
                {t('upgrade.cancel')}
              </Button>
              <Button onClick={runBackup}>{t('upgrade.retryBackup')}</Button>
            </>
          )}
          {step === "upgrading" && error && (
            <Button variant="outline" onClick={() => handleClose(false)}>
              {t('upgrade.close')}
            </Button>
          )}
          {step === "done" && (
            <Button onClick={() => handleClose(false)}>{t('upgrade.close')}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
