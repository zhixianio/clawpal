import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { DownloadIcon } from "lucide-react";
import { toast } from "sonner";

import { api } from "@/lib/api";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";

export function ClawpalLogs() {
  const { t } = useTranslation();
  const [logsTab, setLogsTab] = useState<"app" | "error">("app");
  const [logsContent, setLogsContent] = useState("");
  const [logsLoading, setLogsLoading] = useState(false);
  const [logsError, setLogsError] = useState("");
  const logsContentRef = useRef<HTMLPreElement>(null);

  const fetchLog = useCallback((which: "app" | "error") => {
    setLogsLoading(true);
    setLogsError("");
    const fn = which === "app" ? api.readAppLog : api.readErrorLog;
    fn(200)
      .then((text) => {
        setLogsContent(text.trim() ? text : t("doctor.noLogs"));
        setTimeout(() => {
          if (logsContentRef.current) {
            logsContentRef.current.scrollTop = logsContentRef.current.scrollHeight;
          }
        }, 50);
      })
      .catch((error) => {
        const text = error instanceof Error ? error.message : String(error);
        setLogsContent("");
        setLogsError(text || t("doctor.noLogs"));
      })
      .finally(() => setLogsLoading(false));
  }, [t]);

  const exportLogs = useCallback(() => {
    try {
      const content = logsContent || logsError || t("doctor.noLogs");
      const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
      const filename = `clawpal-${logsTab}-${timestamp}.log`;
      const blob = new Blob([content], { type: "text/plain" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.style.display = "none";
      anchor.href = url;
      anchor.download = filename;
      document.body.appendChild(anchor);
      anchor.click();
      window.setTimeout(() => {
        document.body.removeChild(anchor);
        URL.revokeObjectURL(url);
      }, 0);
      toast.success(t("doctor.exportLogsSuccess", { filename }));
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      toast.error(t("doctor.exportLogsFailed", { error: text }));
    }
  }, [logsContent, logsError, logsTab, t]);

  useEffect(() => {
    fetchLog(logsTab);
  }, [fetchLog, logsTab]);

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("doctor.clawpalLogs")}</h2>
      <Card className="gap-2 py-4">
        <CardHeader className="pb-0">
          <CardTitle className="text-base">{t("doctor.clawpalLogs")}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="mb-2 flex items-center gap-2 flex-wrap">
            <Button
              variant={logsTab === "app" ? "default" : "outline"}
              size="sm"
              onClick={() => setLogsTab("app")}
            >
              {t("doctor.appLog")}
            </Button>
            <Button
              variant={logsTab === "error" ? "default" : "outline"}
              size="sm"
              onClick={() => setLogsTab("error")}
            >
              {t("doctor.errorLog")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => fetchLog(logsTab)}
              disabled={logsLoading}
            >
              {t("doctor.refreshLogs")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={exportLogs}
              disabled={logsLoading}
            >
              <DownloadIcon className="h-3.5 w-3.5 mr-1.5" />
              {t("doctor.exportLogs")}
            </Button>
          </div>
          {logsError && (
            <p className="mb-2 text-xs text-destructive">
              {t("doctor.logReadFailed", { error: logsError })}
            </p>
          )}
          <pre
            ref={logsContentRef}
            className="min-h-[320px] overflow-auto rounded-md border bg-muted p-3 text-xs font-mono whitespace-pre-wrap break-all"
          >
            {logsContent || t("doctor.noLogs")}
          </pre>
        </CardContent>
      </Card>
    </section>
  );
}
