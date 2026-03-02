import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

const AVAILABLE_MODELS = [
  "gpt-4o",
  "gpt-4o-mini",
  "gpt-4.1",
  "claude-3.7-sonnet",
  "claude-3.5-haiku",
  "gemini-2.0-flash",
  "kimi-k2.5",
];

interface ModelSwitcherProps {
  sessionId: string;
  defaultModel?: string;
  /** Notifies parent when the effective model changes (override set/cleared). */
  onModelChange?: (model: string | undefined) => void;
}

export function ModelSwitcher({ sessionId, defaultModel, onModelChange }: ModelSwitcherProps) {
  const [override, setOverride] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!sessionId) return;
    invoke<string | null>("get_session_model_override", { sessionId })
      .then(setOverride)
      .catch(() => {});
  }, [sessionId]);

  const currentModel = override ?? defaultModel ?? "auto";

  const handleSelect = async (model: string) => {
    try {
      await invoke("set_session_model_override", { sessionId, model });
      setOverride(model);
      onModelChange?.(model);
    } catch {
      // silently ignore
    }
    setOpen(false);
  };

  const handleClear = async () => {
    try {
      await invoke("clear_session_model_override", { sessionId });
      setOverride(null);
      onModelChange?.(undefined);
    } catch {
      // silently ignore
    }
    setOpen(false);
  };

  return (
    <div className="flex items-center gap-1.5">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm" className="text-xs h-7 font-mono">
            🧠 {currentModel}
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-56 p-2" align="start">
          <div className="text-xs font-medium text-muted-foreground mb-2">
            Switch model for this session
          </div>
          <div className="space-y-0.5">
            {AVAILABLE_MODELS.map((model) => (
              <button
                key={model}
                className={`w-full text-left text-xs px-2 py-1.5 rounded hover:bg-muted transition-colors ${
                  currentModel === model ? "bg-muted font-medium" : ""
                }`}
                onClick={() => handleSelect(model)}
              >
                {model}
              </button>
            ))}
          </div>
          {override && (
            <div className="mt-2 pt-2 border-t">
              <button
                className="w-full text-left text-xs px-2 py-1.5 rounded text-muted-foreground hover:bg-muted transition-colors"
                onClick={handleClear}
              >
                ↩ Use global default
              </button>
            </div>
          )}
        </PopoverContent>
      </Popover>
      {override && (
        <Badge variant="outline" className="text-[10px] h-5">
          Session override
        </Badge>
      )}
    </div>
  );
}
