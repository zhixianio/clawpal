import { useSyncExternalStore } from "react";
import ReactDiffViewer from "react-diff-viewer-continued";

/** Strip trailing commas from JSON lines so adding a new last property
 *  doesn't cause a spurious diff on the previous line. */
function normalizeJsonForDiff(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/,(\s*)$/, "$1"))
    .join("\n");
}

const DARK_MQ = "(prefers-color-scheme: dark)";

function getIsDark(): boolean {
  return document.documentElement.classList.contains("dark");
}

function subscribeDark(cb: () => void) {
  const mq = window.matchMedia(DARK_MQ);
  const observer = new MutationObserver(cb);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
  mq.addEventListener("change", cb);
  return () => {
    observer.disconnect();
    mq.removeEventListener("change", cb);
  };
}

export function DiffViewer({
  oldValue,
  newValue,
}: {
  oldValue: string;
  newValue: string;
}) {
  const isDark = useSyncExternalStore(subscribeDark, getIsDark, () => false);

  return (
    <div className="max-h-[400px] overflow-auto rounded-lg border">
      <ReactDiffViewer
        oldValue={normalizeJsonForDiff(oldValue)}
        newValue={normalizeJsonForDiff(newValue)}
        splitView={false}
        hideLineNumbers={false}
        showDiffOnly={true}
        extraLinesSurroundingDiff={3}
        useDarkTheme={isDark}
      />
    </div>
  );
}
