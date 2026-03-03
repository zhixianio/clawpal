import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useApi } from "@/lib/use-api";
import type { BugReportSettings as BugReportSettingsModel, BugReportStats } from "@/lib/types";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

const PRIVACY_POLICY_URL = "https://clawpal.zhixian.io/privacy";

function normalizeSettings(input: BugReportSettingsModel): BugReportSettingsModel {
  return {
    ...input,
    endpoint: input.endpoint?.trim() || null,
    maxReportsPerHour: Math.max(1, Math.min(1000, Math.floor(input.maxReportsPerHour || 1))),
  };
}

export function BugReportSettings() {
  const { t } = useTranslation();
  const ua = useApi();
  const [settings, setSettings] = useState<BugReportSettingsModel | null>(null);
  const [stats, setStats] = useState<BugReportStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([ua.getBugReportSettings(), ua.getBugReportStats()])
      .then(([nextSettings, nextStats]) => {
        if (cancelled) return;
        setSettings(nextSettings);
        setStats(nextStats);
      })
      .catch((error) => {
        if (cancelled) return;
        const errorText = error instanceof Error ? error.message : String(error);
        toast.error(t("settings.bugReportLoadFailed", { error: errorText }));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [ua, t]);

  const save = async () => {
    if (!settings) return;
    setSaving(true);
    try {
      const normalized = normalizeSettings(settings);
      const saved = await ua.setBugReportSettings(normalized);
      setSettings(saved);
      const latestStats = await ua.getBugReportStats();
      setStats(latestStats);
      toast.success(t("settings.bugReportSaved"));
    } catch (error) {
      const errorText = error instanceof Error ? error.message : String(error);
      toast.error(t("settings.bugReportSaveFailed", { error: errorText }));
    } finally {
      setSaving(false);
    }
  };

  const sendTestReport = async () => {
    setTesting(true);
    try {
      await ua.testBugReportConnection();
      const latestStats = await ua.getBugReportStats();
      setStats(latestStats);
      toast.success(t("settings.bugReportTestSuccess"));
    } catch (error) {
      const errorText = error instanceof Error ? error.message : String(error);
      toast.error(t("settings.bugReportTestFailed", { error: errorText }));
    } finally {
      setTesting(false);
    }
  };

  const update = (patch: Partial<BugReportSettingsModel>) => {
    setSettings((prev) => (prev ? { ...prev, ...patch } : prev));
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.bugReportTitle")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {loading || !settings ? (
          <p className="text-sm text-muted-foreground">{t("settings.loading")}</p>
        ) : (
          <>
            <div className="flex items-center gap-2">
              <Checkbox
                id="bug-report-enabled"
                checked={settings.enabled}
                onCheckedChange={(checked) => update({ enabled: checked === true })}
              />
              <Label htmlFor="bug-report-enabled">{t("settings.bugReportToggle")}</Label>
            </div>

            <div className="grid gap-3 md:grid-cols-2">
              <div className="space-y-1.5">
                <Label>{t("settings.bugReportBackend")}</Label>
                <Select
                  value={settings.backend}
                  onValueChange={(backend) => update({ backend: backend as BugReportSettingsModel["backend"] })}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="sentry">Sentry</SelectItem>
                    <SelectItem value="glitchTip">GlitchTip</SelectItem>
                    <SelectItem value="customUrl">{t("settings.bugReportBackendCustom")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <Label>{t("settings.bugReportSeverityThreshold")}</Label>
                <Select
                  value={settings.severityThreshold}
                  onValueChange={(severityThreshold) =>
                    update({ severityThreshold: severityThreshold as BugReportSettingsModel["severityThreshold"] })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="error">{t("settings.bugReportSeverityError")}</SelectItem>
                    <SelectItem value="critical">{t("settings.bugReportSeverityCritical")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            {settings.backend !== "sentry" && (
              <div className="space-y-1.5">
                <Label>{t("settings.bugReportEndpoint")}</Label>
                <Input
                  value={settings.endpoint || ""}
                  onChange={(event) => update({ endpoint: event.target.value || null })}
                  placeholder={
                    settings.backend === "customUrl"
                      ? "https://example.com/bug-report"
                      : "https://public-key@glitchtip.example.com/1"
                  }
                />
              </div>
            )}

            {settings.backend === "sentry" && (
              <p className="text-xs text-muted-foreground">{t("settings.bugReportSentryBuiltin")}</p>
            )}

            <div className="space-y-1.5 max-w-[220px]">
              <Label>{t("settings.bugReportRateLimit")}</Label>
              <Input
                type="number"
                min={1}
                max={1000}
                value={settings.maxReportsPerHour}
                onChange={(event) => {
                  const next = Number.parseInt(event.target.value, 10);
                  update({
                    maxReportsPerHour: Number.isFinite(next) ? next : settings.maxReportsPerHour,
                  });
                }}
              />
            </div>

            <p className="text-xs text-muted-foreground">{t("settings.bugReportDescription")}</p>
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" variant="outline" onClick={save} disabled={saving}>
                {saving ? t("settings.saving") : t("settings.save")}
              </Button>
              <Button size="sm" variant="outline" onClick={sendTestReport} disabled={testing || !settings.enabled}>
                {testing ? t("settings.testing") : t("settings.bugReportTest")}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => ua.openUrl(PRIVACY_POLICY_URL)}>
                {t("settings.bugReportPrivacyPolicy")}
              </Button>
            </div>

            {stats && (
              <div className="text-xs text-muted-foreground grid gap-1">
                <p>{t("settings.bugReportStatsSent", { count: stats.totalSent })}</p>
                <p>{t("settings.bugReportStatsLastHour", { count: stats.sentLastHour })}</p>
                <p>{t("settings.bugReportStatsDropped", { count: stats.droppedRateLimited })}</p>
                <p>{t("settings.bugReportStatsLastSent", { value: stats.lastSentAt || "-" })}</p>
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

