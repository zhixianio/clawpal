import { useState } from "react";
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
  const [step, setStep] = useState<Step>("confirm");
  const [backupName, setBackupName] = useState("");
  const [output, setOutput] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const reset = () => {
    setStep("confirm");
    setBackupName("");
    setOutput("");
    setError("");
    setLoading(false);
  };

  const handleClose = (open: boolean) => {
    if (!open && !loading) {
      reset();
      onOpenChange(false);
    }
  };

  const startUpgrade = async () => {
    if (isRemote) {
      setStep("upgrading");
      runUpgrade();
    } else {
      setStep("backup");
      runBackup();
    }
  };

  const runBackup = async () => {
    setLoading(true);
    setError("");
    try {
      const info = await api.backupBeforeUpgrade();
      setBackupName(info.name);
      setLoading(false);
      setStep("upgrading");
      runUpgrade();
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
      setOutput(result);
      setStep("done");
    } catch (e) {
      setOutput(String(e));
      setError("Upgrade failed. See output below.");
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {step === "confirm" && "Upgrade OpenClaw"}
            {step === "backup" && "Creating Backup..."}
            {step === "upgrading" && "Upgrading..."}
            {step === "done" && "Upgrade Complete"}
          </DialogTitle>
        </DialogHeader>

        {step === "confirm" && (
          <div className="space-y-3">
            <div className="flex items-center gap-2 text-sm">
              <span className="text-muted-foreground">Current:</span>
              <code className="font-medium">{currentVersion}</code>
              <span className="text-muted-foreground mx-1">&rarr;</span>
              <span className="text-muted-foreground">New:</span>
              <code className="font-medium text-primary">{latestVersion}</code>
            </div>
            <p className="text-sm text-muted-foreground">
              {isRemote
                ? "This will upgrade OpenClaw on the remote instance."
                : "This will back up your config and upgrade OpenClaw."}
            </p>
          </div>
        )}

        {step === "backup" && (
          <div className="space-y-3">
            {loading && (
              <div className="flex items-center gap-2 text-sm">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span>Creating backup...</span>
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
                Backup created: <code>{backupName}</code>
              </p>
            )}
            {loading && (
              <div className="flex items-center gap-2 text-sm">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span>Running upgrade...</span>
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
                Backup: <code>{backupName}</code>
              </p>
            )}
            <p className="text-sm font-medium text-green-600">
              Upgrade completed successfully.
            </p>
            {output && (
              <pre className="max-h-60 overflow-auto rounded-md bg-muted p-3 text-xs font-mono whitespace-pre-wrap">
                {output}
              </pre>
            )}
          </div>
        )}

        <DialogFooter>
          {step === "confirm" && (
            <>
              <Button variant="outline" onClick={() => handleClose(false)}>
                Cancel
              </Button>
              <Button onClick={startUpgrade}>Start Upgrade</Button>
            </>
          )}
          {step === "backup" && error && (
            <>
              <Button variant="outline" onClick={() => handleClose(false)}>
                Cancel
              </Button>
              <Button onClick={runBackup}>Retry Backup</Button>
            </>
          )}
          {step === "upgrading" && error && (
            <Button variant="outline" onClick={() => handleClose(false)}>
              Close
            </Button>
          )}
          {step === "done" && (
            <Button onClick={() => handleClose(false)}>Close</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
