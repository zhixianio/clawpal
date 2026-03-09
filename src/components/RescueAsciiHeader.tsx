import { useEffect, useState } from "react";

import type { RescueBotRuntimeState } from "@/lib/types";
import { cn } from "@/lib/utils";

const BOT_MATRIX = [
  "......bb......bb......",
  ".....bccb....bccb.....",
  ".....bbbbbbbbbbbb.....",
  "....bbbbbbbbbbbbbb....",
  "...bbb..........bbb...",
  "..bbbb..e....e..bbbb..",
  "..bbbb..........bbbb..",
  "..bbbb..........bbbb..",
  "...bbb..........bbb...",
  "....bbbbbbbbbbbbbb....",
  ".....tttttttttttt.....",
  "......................",
  "......................",
] as const;

const GRID_WIDTH = BOT_MATRIX[0].length;
const PROGRESS_SLOTS = 12;

type BotCellToken = "." | "b" | "c" | "e" | "l" | "t";

const bodyToneByState: Record<RescueBotRuntimeState, string> = {
  unconfigured: "bg-[#B8A696]",
  configured_inactive: "bg-[#8A796B]",
  active: "bg-[#7A695B]",
  checking: "bg-[#7A695B]",
  error: "bg-[#9A6A5A]",
};

const crownToneByState: Record<RescueBotRuntimeState, string> = {
  unconfigured: "bg-[#D8C7B5]",
  configured_inactive: "bg-[#C9B39B]",
  active: "bg-[#E9DED2]",
  checking: "bg-[#E9DED2]",
  error: "bg-[#E5C9BE]",
};

const eyeToneByState: Record<RescueBotRuntimeState, string> = {
  unconfigured: "bg-[#8B5E34]",
  configured_inactive: "bg-[#8B5E34]",
  active: "bg-[#C97A1A]",
  checking: "bg-[#C97A1A]",
  error: "bg-[#C65A3A]",
};

const progressFillToneByState: Record<RescueBotRuntimeState, string> = {
  unconfigured: "bg-[#B38A54]",
  configured_inactive: "bg-[#B38A54]",
  active: "bg-[#78A287]",
  checking: "bg-[#C97A1A]",
  error: "bg-[#C65A3A]",
};

interface RescueAsciiHeaderProps {
  state: RescueBotRuntimeState;
  title: string;
  progress?: number;
  animateProgress?: boolean;
  animateFace?: boolean;
}

function clampProgress(progress?: number): number {
  if (typeof progress !== "number" || Number.isNaN(progress)) {
    return 0;
  }
  return Math.max(0, Math.min(1, progress));
}

function cellLabel(token: BotCellToken, progressIndex: number, filledSlots: number) {
  switch (token) {
    case "b":
    case "c":
      return "body";
    case "e":
      return "eye";
    case "l":
      return "leg";
    case "t":
      return progressIndex < filledSlots ? "progress-fill" : "progress-empty";
    default:
      return "empty";
  }
}

export function RescueAsciiHeader({
  state,
  title,
  progress,
  animateProgress = false,
  animateFace = false,
}: RescueAsciiHeaderProps) {
  const clampedProgress = clampProgress(progress);
  const filledSlots = Math.round(clampedProgress * PROGRESS_SLOTS);
  const [blinkClosed, setBlinkClosed] = useState(false);
  let progressIndex = 0;

  useEffect(() => {
    if (!animateFace) {
      setBlinkClosed(false);
      return;
    }
    let closedTimeout: number | null = null;
    const interval = window.setInterval(() => {
      setBlinkClosed(true);
      closedTimeout = window.setTimeout(() => {
        setBlinkClosed(false);
      }, 180);
    }, 2200);
    return () => {
      window.clearInterval(interval);
      if (closedTimeout !== null) {
        window.clearTimeout(closedTimeout);
      }
    };
  }, [animateFace]);

  return (
    <div className="min-w-0 text-center">
      <div
        role="img"
        aria-label={title}
        title={title}
        data-led-bot="wide-console"
        className="inline-grid gap-[1px] justify-center"
        style={{ gridTemplateColumns: `repeat(${GRID_WIDTH}, minmax(0, 1fr))` }}
      >
        {BOT_MATRIX.flatMap((row, rowIndex) =>
          row.split("").map((token, columnIndex) => {
            const typedToken = token as BotCellToken;
            const label = cellLabel(typedToken, progressIndex, filledSlots);
            const isFilledProgress = typedToken === "t" && progressIndex < filledSlots;
            const isProgress = typedToken === "t";
            if (typedToken === "t") {
              progressIndex += 1;
            }

            return (
              <span
                key={`${rowIndex}-${columnIndex}`}
                data-bot-cell={label}
                aria-hidden="true"
                className={cn(
                  "inline-flex h-[10px] w-[10px] items-center justify-center sm:h-[12px] sm:w-[12px]",
                  typedToken === "." && "opacity-0",
                  typedToken === "b" && bodyToneByState[state],
                  typedToken === "c" && crownToneByState[state],
                  typedToken === "l" && "bg-[#C9B8A6]",
                  isProgress && "bg-[#E9DED2]",
                  isFilledProgress &&
                    cn(
                      progressFillToneByState[state],
                      animateProgress && "animate-pulse transition-colors duration-300",
                    ),
                )}
              >
                {typedToken === "e" ? (
                  blinkClosed ? (
                    <span
                      data-bot-eye-expression="blink"
                      className={cn(
                        "h-[2px] w-[10px] rounded-full sm:w-[11px]",
                        eyeToneByState[state],
                      )}
                    />
                  ) : state === "active" ? (
                    <span
                      data-bot-eye-expression="uparrow"
                      className="relative translate-y-[2px] h-[14px] w-[20px] sm:h-[16px] sm:w-[22px]"
                    >
                      <span
                        className={cn(
                          "absolute left-[2px] top-[6px] h-[3px] w-[10px] origin-right rotate-45 rounded-full sm:left-[2px] sm:top-[6px] sm:w-[11px]",
                          eyeToneByState[state],
                        )}
                      />
                      <span
                        className={cn(
                          "absolute right-[2px] top-[6px] h-[3px] w-[10px] origin-left -rotate-45 rounded-full sm:right-[2px] sm:top-[6px] sm:w-[11px]",
                          eyeToneByState[state],
                        )}
                      />
                    </span>
                  ) : (
                    <span
                      data-bot-eye-expression="idle"
                      className={cn(
                        "h-[5px] w-[5px] rounded-full sm:h-[6px] sm:w-[6px]",
                        eyeToneByState[state],
                        state === "checking" && "animate-pulse",
                      )}
                    />
                  )
                ) : null}
              </span>
            );
          }),
        )}
      </div>
    </div>
  );
}
