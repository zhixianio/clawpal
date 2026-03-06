import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { XIcon } from "lucide-react";
import { DoctorChat } from "@/components/DoctorChat";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useInstance } from "@/lib/instance-context";
import { useDoctorAgent } from "@/lib/use-doctor-agent";
import {
  getQuickDiagnoseTransport,
  buildPrefillMessage,
  shouldSeedContext,
  handleQuickDiagnoseDialogOpenChange,
} from "@/components/quick-diagnose-utils";


interface QuickDiagnoseDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  context?: string | null;
}

export function QuickDiagnoseDialog({
  open,
  onOpenChange,
  context = null,
}: QuickDiagnoseDialogProps) {
  const { t } = useTranslation();
  const {
    messages,
    loading,
    error,
    connected,
    connect,
    disconnect,
    startDiagnosis,
    sendMessage,
    approveInvoke,
    rejectInvoke,
    setTarget,
    reset,
  } = useDoctorAgent();
  const { instanceId, isRemote, isDocker } = useInstance();
  const [bootstrapping, setBootstrapping] = useState(false);
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const seededRef = useRef<string>("");
  const transport = useMemo(() => getQuickDiagnoseTransport(isRemote, isDocker), [isDocker, isRemote]);

  const handleOpenChange = useCallback((nextOpen: boolean) => {
    handleQuickDiagnoseDialogOpenChange(onOpenChange, nextOpen);
  }, [onOpenChange]);

  const handleClose = useCallback(() => {
    handleOpenChange(false);
  }, [handleOpenChange]);

  useEffect(() => {
    if (!open) return;
    seededRef.current = "";
    setBootstrapError(null);
    setBootstrapping(true);

    let cancelled = false;
    const initialContext = buildPrefillMessage(context);

    const start = async () => {
      reset();
      setTarget(transport === "remote_ssh" ? instanceId : "local");
      await connect();
      await startDiagnosis(
        t("quickDiagnose.placeholder"),
        "main",
        instanceId,
        transport,
        undefined,
        "doctor",
        "zeroclaw",
      );
      if (shouldSeedContext(initialContext, seededRef.current)) {
        seededRef.current = initialContext;
        await sendMessage(initialContext);
      }
    };

    void start()
      .catch((err) => {
        if (!cancelled) {
          setBootstrapError(String(err));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setBootstrapping(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [connect, context, instanceId, open, reset, sendMessage, setTarget, startDiagnosis, t, transport]);

  useEffect(() => {
    if (open) return;
    void disconnect();
    reset();
    setBootstrapping(false);
    setBootstrapError(null);
  }, [disconnect, open, reset]);

  if (!open) return null;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <div className="flex items-center justify-between gap-3">
            <DialogTitle>{t("quickDiagnose.title")}</DialogTitle>
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={handleClose}
              aria-label={t("config.close")}
            >
              <XIcon className="size-4" />
            </Button>
          </div>
        </DialogHeader>
        {(bootstrapError || error) && (
          <div className="text-sm text-destructive">
            {bootstrapError || error}
          </div>
        )}
        {bootstrapping && messages.length === 0 && (
          <div className="text-sm text-muted-foreground animate-pulse">
            {t("doctor.connecting")}
          </div>
        )}
        <DoctorChat
          messages={messages}
          loading={loading || bootstrapping}
          error={error}
          connected={connected}
          onSendMessage={sendMessage}
          onApproveInvoke={approveInvoke}
          onRejectInvoke={rejectInvoke}
        />
      </DialogContent>
    </Dialog>
  );
}
