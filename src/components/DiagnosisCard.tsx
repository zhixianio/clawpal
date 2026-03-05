import { useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardCopyIcon, DownloadIcon } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { DiagnosisReportItem } from "@/lib/types";

interface DiagnosisCardProps {
  items: DiagnosisReportItem[];
}

interface DiagnosisCardViewProps {
  items: DiagnosisReportItem[];
  t: ReturnType<typeof useTranslation>["t"];
}

const severityConfig = {
  error: { label: "ERROR", variant: "destructive" as const, border: "border-l-destructive" },
  warn: { label: "WARN", variant: "secondary" as const, border: "border-l-yellow-500" },
  info: { label: "INFO", variant: "outline" as const, border: "border-l-blue-500" },
};

export function formatConfidence(value?: number): string | null {
  if (typeof value !== "number" || Number.isNaN(value)) return null;
  const clamped = Math.max(0, Math.min(1, value));
  return `${Math.round(clamped * 100)}%`;
}

export function formatMarkdown(items: DiagnosisReportItem[]): string {
  return items
    .map((item, i) => {
      const sev = item.severity.toUpperCase();
      const lines = [`## ${i + 1}. [${sev}] ${item.problem}`];
      if (item.root_cause_hypothesis) {
        lines.push("", `**Root cause hypothesis:** ${item.root_cause_hypothesis}`);
      }
      const confidenceText = formatConfidence(item.confidence);
      if (confidenceText) {
        lines.push("", `**Confidence:** ${confidenceText}`);
      }
      const steps = item.fix_steps && item.fix_steps.length > 0 ? item.fix_steps : item.fix_options;
      if (steps.length > 0) {
        lines.push("", "**Fix steps:**", ...steps.map((o) => `- ${o}`));
      }
      if (item.citations && item.citations.length > 0) {
        lines.push(
          "",
          "**Citations:**",
          ...item.citations.map((citation) => `- ${citation.url}${citation.section ? ` (${citation.section})` : ""}`),
        );
      }
      if (item.version_awareness) {
        lines.push("", `**Version awareness:** ${item.version_awareness}`);
      }
      return lines.join("\n");
    })
    .join("\n\n");
}

export function formatJson(items: DiagnosisReportItem[]): string {
  return JSON.stringify(items, null, 2);
}

export function toggleCheckedState(prev: Record<number, boolean>, idx: number): Record<number, boolean> {
  return { ...prev, [idx]: !prev[idx] };
}

export async function handleDiagnosisExport(
  items: DiagnosisReportItem[],
  format: "markdown" | "json",
  writeText: (text: string) => Promise<void>,
  setCopied: (value: boolean) => void,
  setExportOpen: (value: boolean) => void,
  scheduleClearCopied: (callback: () => void, delayMs: number) => unknown = setTimeout,
): Promise<void> {
  const text = format === "json" ? formatJson(items) : formatMarkdown(items);
  setExportOpen(false);
  await writeText(text);
  setCopied(true);
  scheduleClearCopied(() => setCopied(false), 1500);
}

export function safeClipboardWrite(text: string): Promise<void> {
  const writer = navigator?.clipboard?.writeText;
  if (typeof writer === "function") {
    return writer.call(navigator.clipboard, text);
  }
  return Promise.resolve();
}

export function toggleExportMenu(currentOpen: boolean, setExportOpen: (value: boolean) => void): void {
  setExportOpen(!currentOpen);
}

export function applyCheckedToggle(
  idx: number,
  setChecked: Dispatch<SetStateAction<Record<number, boolean>>>,
): void {
  setChecked((prev) => toggleCheckedState(prev, idx));
}

export function DiagnosisCardView({ items, t }: DiagnosisCardViewProps) {
  const [checked, setChecked] = useState<Record<number, boolean>>({});
  const [exportOpen, setExportOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const writeToClipboard = safeClipboardWrite;

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-muted-foreground">
          {t("doctor.diagnosisReport", { defaultValue: "Diagnosis Report" })} ({items.length})
        </span>
        <div className="relative">
          <Button
            variant="ghost"
            size="xs"
            onClick={toggleExportMenu.bind(null, exportOpen, setExportOpen)}
          >
            <DownloadIcon className="size-3.5 mr-1" />
            {copied
              ? t("doctor.copied", { defaultValue: "Copied!" })
              : t("doctor.export", { defaultValue: "Export" })}
          </Button>
          {exportOpen && (
            <div className="absolute right-0 top-full mt-1 z-10 rounded-md border bg-popover p-1 shadow-md min-w-[120px]">
              <button
                className="w-full text-left text-xs px-2 py-1.5 rounded hover:bg-accent"
                onClick={() =>
                  void handleDiagnosisExport(
                    items,
                    "markdown",
                    writeToClipboard,
                    setCopied,
                    setExportOpen,
                  )
                }
              >
                <ClipboardCopyIcon className="size-3 inline mr-1.5" />
                Markdown
              </button>
              <button
                className="w-full text-left text-xs px-2 py-1.5 rounded hover:bg-accent"
                onClick={() =>
                  void handleDiagnosisExport(
                    items,
                    "json",
                    writeToClipboard,
                    setCopied,
                    setExportOpen,
                  )
                }
              >
                <ClipboardCopyIcon className="size-3 inline mr-1.5" />
                JSON
              </button>
            </div>
          )}
        </div>
      </div>

      {items.map((item, idx) => {
        const cfg = severityConfig[item.severity] ?? severityConfig.info;
        const steps = item.fix_steps && item.fix_steps.length > 0 ? item.fix_steps : item.fix_options;
        const citations = item.citations ?? [];
        const confidenceText = formatConfidence(item.confidence);
        return (
          <Card
            key={idx}
            className={`border-l-[3px] ${cfg.border} bg-[oklch(0.96_0_0)] dark:bg-muted/50 py-3`}
          >
            <CardContent className="px-4 py-0 space-y-2">
              <div className="flex items-start gap-2">
                <Checkbox
                  checked={!!checked[idx]}
                  onCheckedChange={applyCheckedToggle.bind(null, idx, setChecked)}
                  className="mt-0.5"
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <Badge variant={cfg.variant} className="text-[10px] px-1.5 py-0">
                      {cfg.label}
                    </Badge>
                    {confidenceText && (
                      <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                        {confidenceText}
                      </Badge>
                    )}
                    <span className="text-sm font-medium">{item.problem}</span>
                  </div>
                  {item.root_cause_hypothesis && (
                    <div className="mt-1.5 text-xs text-muted-foreground">
                      {item.root_cause_hypothesis}
                    </div>
                  )}
                  {steps.length > 0 && (
                    <ul className="mt-1.5 space-y-0.5">
                      {steps.map((opt, oi) => (
                        <li key={oi} className="text-xs text-muted-foreground flex gap-1.5">
                          <span className="text-muted-foreground/60">•</span>
                          {opt}
                        </li>
                      ))}
                    </ul>
                  )}
                  {citations.length > 0 && (
                    <div className="mt-2 space-y-0.5">
                      {citations.map((citation, ci) => (
                        <a
                          key={`${citation.url}-${ci}`}
                          href={citation.url}
                          target="_blank"
                          rel="noreferrer"
                          className="block text-[11px] text-blue-600 hover:underline dark:text-blue-400 break-all"
                        >
                          {citation.section ? `${citation.section} · ` : ""}
                          {citation.url}
                        </a>
                      ))}
                    </div>
                  )}
                  {item.version_awareness && (
                    <div className="mt-2 text-[11px] text-muted-foreground">
                      {item.version_awareness}
                    </div>
                  )}
                </div>
                {item.action && (
                  <Button variant="outline" size="xs" className="shrink-0">
                    {t("doctor.autoFix", { defaultValue: "Auto-fix" })}
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>
        );
      })}
    </div>
  );
}

export function DiagnosisCard({ items }: DiagnosisCardProps) {
  const { t } = useTranslation();
  return <DiagnosisCardView items={items} t={t} />;
}
