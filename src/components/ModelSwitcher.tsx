import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

interface ModelSwitcherProps {
  sessionId: string;
  defaultModel?: string;
  availableModels?: string[];
  /** Notifies parent when the effective model changes (override set/cleared). */
  onModelChange?: (model: string | undefined) => void;
}

export function ModelSwitcher({ sessionId, defaultModel, availableModels, onModelChange }: ModelSwitcherProps) {
  const [override, setOverride] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!sessionId) return;
    invoke<string | null>("get_session_model_override", { sessionId })
      .then(setOverride)
      .catch(() => {});
  }, [sessionId]);

  const currentModel = override ?? defaultModel ?? "auto";

  const models = useMemo(() => {
    const uniqueModels = new Map<string, string>();
    for (const model of availableModels || []) {
      const normalized = model.trim();
      if (!normalized) continue;
      const key = normalized.toLowerCase();
      if (!uniqueModels.has(key)) uniqueModels.set(key, normalized);
    }
    return Array.from(uniqueModels.values()).sort((a, b) => a.localeCompare(b));
  }, [availableModels]);

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
            {models.length === 0 ? (
              <div className="text-xs text-muted-foreground px-2 py-2">
                No available model profiles configured
              </div>
            ) : (
              models.map((model) => (
                <button
                  key={model}
                  className={`w-full text-left text-xs px-2 py-1.5 rounded hover:bg-muted transition-colors ${
                    currentModel === model ? "bg-muted font-medium" : ""
                  }`}
                  onClick={() => handleSelect(model)}
                >
                  {model}
                </button>
              ))
            )}
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
