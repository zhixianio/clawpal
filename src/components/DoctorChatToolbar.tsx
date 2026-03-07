import { useTranslation } from "react-i18next";
import { EraserIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

interface DoctorChatToolbarProps {
  fullAuto: boolean;
  clearDisabled?: boolean;
  onFullAutoChange: (checked: boolean) => void;
  onClear: () => void;
}

export function DoctorChatToolbar({
  fullAuto,
  clearDisabled = false,
  onFullAutoChange,
  onClear,
}: DoctorChatToolbarProps) {
  const { t } = useTranslation();
  const clearLabel = t("doctor.clear");

  return (
    <div className="mb-3 flex items-center justify-end gap-2">
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={onClear}
        disabled={clearDisabled}
        aria-label={clearLabel}
        title={clearLabel}
        className="text-muted-foreground hover:text-foreground"
      >
        <EraserIcon className="size-3.5" />
      </Button>
      <label className="flex items-center gap-1.5 text-xs cursor-pointer select-none">
        <input
          type="checkbox"
          checked={fullAuto}
          onChange={(e) => onFullAutoChange(e.target.checked)}
          className="accent-primary"
        />
        {t("doctor.fullAuto")}
      </label>
    </div>
  );
}
