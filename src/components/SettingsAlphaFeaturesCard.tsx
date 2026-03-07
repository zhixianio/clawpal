import { useTranslation } from "react-i18next";

import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { DisclosureCard } from "@/components/DisclosureCard";

interface SettingsAlphaFeaturesCardProps {
  showSshTransferSpeedUi: boolean;
  showClawpalLogsUi: boolean;
  showGatewayLogsUi: boolean;
  showOpenclawContextUi: boolean;
  onSshTransferSpeedUiToggle: (checked: boolean) => void;
  onClawpalLogsUiToggle: (checked: boolean) => void;
  onGatewayLogsUiToggle: (checked: boolean) => void;
  onOpenclawContextUiToggle: (checked: boolean) => void;
}

export function SettingsAlphaFeaturesCard({
  showSshTransferSpeedUi,
  showClawpalLogsUi,
  showGatewayLogsUi,
  showOpenclawContextUi,
  onSshTransferSpeedUiToggle,
  onClawpalLogsUiToggle,
  onGatewayLogsUiToggle,
  onOpenclawContextUiToggle,
}: SettingsAlphaFeaturesCardProps) {
  const { t } = useTranslation();

  return (
    <DisclosureCard
      title={t("settings.alphaFeatures")}
      description={t("settings.alphaFeaturesDescription")}
    >
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <Label className="text-sm font-medium">{t("settings.alphaEnableSshTransferSpeedUi")}</Label>
        <Checkbox
          checked={showSshTransferSpeedUi}
          onCheckedChange={(checked) => onSshTransferSpeedUiToggle(checked === true)}
          aria-label={t("settings.alphaEnableSshTransferSpeedUi")}
          className="h-5 w-5"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.alphaEnableSshTransferSpeedUiHint")}
      </p>

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <Label className="text-sm font-medium">{t("settings.alphaEnableClawpalLogsUi")}</Label>
        <Checkbox
          checked={showClawpalLogsUi}
          onCheckedChange={(checked) => onClawpalLogsUiToggle(checked === true)}
          aria-label={t("settings.alphaEnableClawpalLogsUi")}
          className="h-5 w-5"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.alphaEnableClawpalLogsUiHint")}
      </p>

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <Label className="text-sm font-medium">{t("settings.alphaEnableGatewayLogsUi")}</Label>
        <Checkbox
          checked={showGatewayLogsUi}
          onCheckedChange={(checked) => onGatewayLogsUiToggle(checked === true)}
          aria-label={t("settings.alphaEnableGatewayLogsUi")}
          className="h-5 w-5"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.alphaEnableGatewayLogsUiHint")}
      </p>

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <Label className="text-sm font-medium">{t("settings.alphaEnableOpenclawContextUi")}</Label>
        <Checkbox
          checked={showOpenclawContextUi}
          onCheckedChange={(checked) => onOpenclawContextUiToggle(checked === true)}
          aria-label={t("settings.alphaEnableOpenclawContextUi")}
          className="h-5 w-5"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.alphaEnableOpenclawContextUiHint")}
      </p>
    </DisclosureCard>
  );
}
