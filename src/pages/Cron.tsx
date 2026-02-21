import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import type {
  CronJob,
  CronRun,
  CronSchedule,
  WatchdogStatus,
} from "@/lib/types";
import {
  Card,
  CardContent,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

const DOW_EN = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DOW_ZH = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

function cronToHuman(expr: string, t: TFunction, lang: string): string {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return expr;
  const [min, hour, dom, mon, dow] = parts;
  const time = `${hour.padStart(2, "0")}:${min.padStart(2, "0")}`;
  const dowNames = lang.startsWith("zh") ? DOW_ZH : DOW_EN;

  if (min.startsWith("*/") && hour === "*" && dom === "*" && mon === "*" && dow === "*")
    return t("cron.every", { interval: `${min.slice(2)}m` });
  if (min === "0" && hour.startsWith("*/") && dom === "*" && mon === "*" && dow === "*")
    return t("cron.every", { interval: `${hour.slice(2)}h` });
  if (dom === "*" && mon === "*" && dow !== "*" && !hour.includes("/") && !min.includes("/")) {
    const days = dow.split(",").map(d => dowNames[parseInt(d)] || d).join(", ");
    return `${days} ${time}`;
  }
  if (dom !== "*" && !dom.includes("/") && mon === "*" && dow === "*" && !hour.includes("/") && !min.includes("/"))
    return t("cron.monthly", { day: dom, time });
  if (dom === "*" && mon === "*" && dow === "*" && !hour.includes("/") && !min.includes("/")) {
    const hours = hour.split(",");
    if (hours.length === 1) return t("cron.daily", { time });
    return t("cron.daily", { time: hours.map(h => `${h.padStart(2, "0")}:${min.padStart(2, "0")}`).join(", ") });
  }
  return expr;
}

function formatSchedule(s: CronSchedule | undefined, t: TFunction, lang: string): string {
  if (!s) return "—";
  if (s.kind === "every" && s.everyMs) {
    const mins = Math.round(s.everyMs / 60000);
    return mins >= 60 ? t("cron.every", { interval: `${Math.round(mins / 60)}h` }) : t("cron.every", { interval: `${mins}m` });
  }
  if (s.kind === "at" && s.at) return fmtDate(new Date(s.at).getTime());
  if (s.kind === "cron" && s.expr) return cronToHuman(s.expr, t, lang);
  return "—";
}

/** YYYY-MM-DD HH:MM:SS */
function fmtDate(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function fmtRelative(ms: number, t: TFunction): string {
  const diff = Date.now() - ms;
  const secs = Math.floor(diff / 1000);
  if (secs < 0) return t("cron.justNow");
  if (secs < 60) return t("cron.secsAgo", { count: secs });
  const mins = Math.floor(secs / 60);
  if (mins < 60) return t("cron.minsAgo", { count: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t("cron.hoursAgo", { count: hours });
  return t("cron.daysAgo", { count: Math.floor(hours / 24) });
}

function fmtDur(ms: number, t: TFunction): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.round(ms / 1000);
  return s < 60 ? t("cron.durSecs", { count: s }) : t("cron.durMins", { m: Math.floor(s / 60), s: s % 60 });
}


/* ------------------------------------------------------------------ */
/*  Trash icon                                                         */
/* ------------------------------------------------------------------ */

const TrashIcon = () => (
  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>
    <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
    <line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/>
  </svg>
);

/* ------------------------------------------------------------------ */
/*  Cron page                                                          */
/* ------------------------------------------------------------------ */

export function Cron() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const ua = useApi();

  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [watchdog, setWatchdog] = useState<(WatchdogStatus & { alive: boolean; deployed: boolean }) | null>(null);
  const [expandedJob, setExpandedJob] = useState<string | null>(null);
  const [runs, setRuns] = useState<Record<string, CronRun[]>>({});
  const [triggering, setTriggering] = useState<string | null>(null);
  const [wdAction, setWdAction] = useState<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [lastSuccess, setLastSuccess] = useState<string | null>(null);

  const loadJobs = useCallback(() => { ua.listCronJobs().then(setJobs).catch(() => {}); }, [ua]);
  const loadWd = useCallback(() => { ua.getWatchdogStatus().then(setWatchdog).catch(() => setWatchdog(null)); }, [ua]);
  const loadRuns = useCallback((id: string) => { ua.getCronRuns(id, 10).then(r => setRuns(p => ({ ...p, [id]: r }))).catch(() => {}); }, [ua]);

  useEffect(() => { loadJobs(); loadWd(); const iv = setInterval(() => { loadJobs(); loadWd(); }, 10_000); return () => clearInterval(iv); }, [loadJobs, loadWd]);
  useEffect(() => { if (expandedJob) loadRuns(expandedJob); }, [expandedJob, loadRuns]);

  const showErr = (e: unknown) => { const msg = e instanceof Error ? e.message : String(e); setLastError(msg); setLastSuccess(null); setTimeout(() => setLastError(null), 8000); };
  const showOk = (msg: string) => { setLastSuccess(msg); setLastError(null); setTimeout(() => setLastSuccess(null), 5000); };

  const doTrigger = (id: string) => {
    setTriggering(id);
    ua.triggerCronJob(id).then(() => { loadJobs(); loadRuns(id); showOk(t("cron.triggerSuccess")); }).catch(showErr).finally(() => setTriggering(null));
  };
  const doDelete = async (id: string) => { try { await ua.deleteCronJob(id); loadJobs(); } catch (e) { showErr(e); } };
  const pollUntilAlive = () => {
    let tries = 0;
    const iv = setInterval(() => {
      tries++;
      ua.getWatchdogStatus()
        .then(s => { setWatchdog(s); if (s?.alive || tries >= 15) { clearInterval(iv); setWdAction(null); } })
        .catch(() => { if (tries >= 15) { clearInterval(iv); setWdAction(null); } });
    }, 2000);
  };
  const doDeploy = async (andStart = false) => {
    setWdAction("deploying");
    try {
      await ua.deployWatchdog();
      if (andStart) {
        setWdAction("starting");
        await ua.startWatchdog();
        pollUntilAlive();
        return; // wdAction cleared by pollUntilAlive
      }
      loadWd();
    } catch (e) { showErr(e); } finally { if (!andStart) setWdAction(null); }
  };
  const doStart = async () => {
    setWdAction("starting");
    try {
      await ua.startWatchdog();
      pollUntilAlive(); // wdAction cleared when alive detected
    } catch (e) { showErr(e); setWdAction(null); }
  };
  const doStop = async () => { setWdAction("stopping"); try { await ua.stopWatchdog(); loadWd(); } catch (e) { showErr(e); } finally { setWdAction(null); } };
  const doUninstall = async () => { setWdAction("uninstalling"); try { await ua.uninstallWatchdog(); loadWd(); } catch (e) { showErr(e); } finally { setWdAction(null); } };

  // watchdog status
  let wdDot = "bg-gray-400", wdText = t("watchdog.notDeployed");
  if (watchdog?.deployed && !watchdog?.alive) { wdDot = "bg-gray-400"; wdText = t("watchdog.stopped"); }
  else if (watchdog?.alive && !watchdog?.lastCheckAt) { wdDot = "bg-yellow-500 animate-pulse"; wdText = t("watchdog.starting"); }
  else if (watchdog?.alive && watchdog?.lastCheckAt) {
    const age = Date.now() - new Date(watchdog.lastCheckAt).getTime();
    if (age <= 120_000) { wdDot = "bg-green-500"; wdText = t("watchdog.running"); }
    else { wdDot = "bg-red-500"; wdText = t("watchdog.crashed"); }
  }

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("cron.title")}</h2>

      {lastError && (
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 px-3 py-2 mb-3 text-xs text-destructive break-words">
          {lastError}
        </div>
      )}
      {lastSuccess && (
        <div className="rounded-lg border border-green-500/30 bg-green-500/10 px-3 py-2 mb-3 text-xs text-green-700 dark:text-green-400">
          {lastSuccess}
        </div>
      )}

      {/* ---- Watchdog ---- */}
      <div className="rounded-lg border bg-card text-card-foreground px-3 py-2 mb-4">
        {/* Row 1: title + button */}
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold">{t("watchdog.title")}</h3>
          <div className="flex items-center gap-1.5 shrink-0">
            {!watchdog?.deployed && <Button size="sm" disabled={!!wdAction} onClick={() => doDeploy(true)}>{wdAction ? t(`watchdog.${wdAction}`) : t("watchdog.deploy")}</Button>}
            {watchdog?.deployed && !watchdog?.alive && !wdAction && (
              <>
                <Button size="sm" variant="ghost" className="text-muted-foreground text-xs" disabled={!!wdAction} onClick={doUninstall}>{t("watchdog.uninstall")}</Button>
                <Button size="sm" variant="outline" disabled={!!wdAction} onClick={() => doDeploy(true)}>{t("watchdog.redeploy")}</Button>
                <Button size="sm" disabled={!!wdAction} onClick={doStart}>{t("watchdog.start")}</Button>
              </>
            )}
            {watchdog?.deployed && !watchdog?.alive && wdAction && (
              <Button size="sm" disabled>{t(`watchdog.${wdAction}`)}</Button>
            )}
            {watchdog?.alive && <Button size="sm" variant="outline" disabled={!!wdAction} onClick={doStop}>{wdAction === "stopping" ? t("watchdog.stopping") : t("watchdog.stop")}</Button>}
          </div>
        </div>
        {/* Row 2: description/status + status indicator */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            {!watchdog?.deployed && (
              <span>{t("watchdog.notDeployedHint")}</span>
            )}
            {watchdog?.deployed && !watchdog?.alive && (
              <span>{t("watchdog.stopped")}</span>
            )}
            {watchdog?.alive && !watchdog?.lastCheckAt && (
              <span>{t("watchdog.startingHint")}</span>
            )}
            {watchdog?.alive && watchdog?.lastCheckAt && (
              <span>{t("watchdog.lastCheck", { time: fmtRelative(new Date(watchdog.lastCheckAt).getTime(), t) })}</span>
            )}
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            <span className={cn("w-2 h-2 rounded-full", wdDot)} />
            <span className="text-xs text-muted-foreground">{wdText}</span>
          </div>
        </div>
      </div>

      {/* ---- Jobs ---- */}
      {jobs.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-muted-foreground">
            <p className="text-sm">{t("cron.noJobs")}</p>
            <p className="text-xs mt-1">{t("cron.noJobsHint")}</p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-1">
          {/* Legend */}
          <div className="flex items-center gap-4 text-[10px] text-muted-foreground px-1 pb-1">
            <span className="flex items-center gap-1"><span className="w-1.5 h-1.5 rounded-full bg-green-500" />{t("cron.legendOk")}</span>
            <span className="flex items-center gap-1"><span className="w-1.5 h-1.5 rounded-full bg-yellow-500" />{t("cron.legendRetrying")}</span>
            <span className="flex items-center gap-1"><span className="w-1.5 h-1.5 rounded-full bg-red-500" />{t("cron.legendEscalated")}</span>
            <span className="flex items-center gap-1"><span className="w-1.5 h-1.5 rounded-full bg-gray-400" />{t("cron.legendDisabled")}</span>
          </div>
          {jobs.map((job, idx) => {
            const jobId = job.jobId || String(idx);
            const st = job.state || {};
            const expanded = expandedJob === jobId;
            const jobName = job.name || jobId;
            const wdStatus = watchdog?.jobs?.[jobId]?.status;

            return (
              <div key={jobId} className={cn("rounded-lg border bg-card text-card-foreground px-3 transition-colors", expanded && "ring-1 ring-ring")}>
                  {/* Single-line row */}
                  <div
                    className="flex items-center gap-3 cursor-pointer h-8"
                    onClick={() => setExpandedJob(p => p === jobId ? null : jobId)}
                  >
                    {/* Status dot */}
                    <span className={cn("w-2 h-2 rounded-full shrink-0",
                      job.enabled === false ? "bg-gray-400"
                        : wdStatus === "escalated" ? "bg-red-500"
                        : wdStatus === "retrying" || wdStatus === "pending" ? "bg-yellow-500"
                        : "bg-green-500"
                    )} />

                    {/* Name */}
                    <span className="text-sm font-medium truncate min-w-0 flex-1">{jobName}</span>

                    {/* Disabled badge */}
                    {job.enabled === false && (
                      <Badge variant="outline" className="text-[10px] font-normal text-muted-foreground px-1 py-0 shrink-0">{t("cron.disabled")}</Badge>
                    )}

                    {/* Delivery error hint */}
                    {job.enabled !== false && st.lastStatus === "error" && (
                      <span className="text-[10px] text-amber-600 dark:text-amber-400 shrink-0" title={st.lastError || ""}>{t("cron.deliveryError")}</span>
                    )}

                    {/* Schedule */}
                    <span className="text-xs text-muted-foreground shrink-0 w-32 text-right truncate">
                      {formatSchedule(job.schedule, t, lang)}
                    </span>

                    {/* Last run */}
                    <span className="text-xs text-muted-foreground shrink-0 w-16 text-right">
                      {st.lastRunAtMs ? fmtRelative(st.lastRunAtMs, t) : "—"}
                    </span>

                    {/* Actions */}
                    <div className="flex items-center gap-1 shrink-0" onClick={e => e.stopPropagation()}>
                      <Button size="xs" variant="outline" disabled={!!triggering || job.enabled === false} onClick={() => doTrigger(jobId)}>
                        {triggering === jobId ? t("cron.triggering") : t("cron.trigger")}
                      </Button>
                      <AlertDialog>
                        <AlertDialogTrigger asChild>
                          <Button size="icon-xs" variant="ghost" className="text-muted-foreground hover:text-destructive"><TrashIcon /></Button>
                        </AlertDialogTrigger>
                        <AlertDialogContent>
                          <AlertDialogHeader>
                            <AlertDialogTitle>{t("cron.deleteTitle")}</AlertDialogTitle>
                            <AlertDialogDescription>{t("cron.deleteDescription", { name: jobName })}</AlertDialogDescription>
                          </AlertDialogHeader>
                          <AlertDialogFooter>
                            <AlertDialogCancel>{t("settings.cancel")}</AlertDialogCancel>
                            <AlertDialogAction className="bg-destructive text-white hover:bg-destructive/90" onClick={() => doDelete(jobId)}>{t("cron.delete")}</AlertDialogAction>
                          </AlertDialogFooter>
                        </AlertDialogContent>
                      </AlertDialog>
                    </div>
                  </div>

                  {/* Expanded run history */}
                  {expanded && (
                    <div className="mt-2 pt-2 border-t border-border">
                      <div className="mb-1.5">
                        <span className="text-xs font-medium text-muted-foreground">{t("cron.runHistory")}</span>
                      </div>
                      {(runs[jobId] || []).length === 0 ? (
                        <p className="text-xs text-muted-foreground py-1">{t("cron.noRuns")}</p>
                      ) : (
                        <div className="space-y-0.5">
                          {(runs[jobId] || []).map((run, i) => {
                            const ts = run.ts || run.runAtMs;
                            return (
                              <div key={i} className="flex items-start gap-3 text-xs py-0.5 min-w-0">
                                <span className="text-muted-foreground w-[130px] shrink-0 tabular-nums">{ts ? fmtDate(ts) : "—"}</span>
                                <span className="text-muted-foreground w-10 shrink-0 tabular-nums">{run.durationMs != null ? fmtDur(run.durationMs, t) : "—"}</span>
                                <span className="text-muted-foreground min-w-0 break-words">
                                  {run.summary || "—"}
                                </span>
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
