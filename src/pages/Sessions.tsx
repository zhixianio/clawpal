import { useTranslation } from "react-i18next";
import { SessionAnalysisPanel } from "@/components/SessionAnalysisPanel";

export function Sessions() {
  const { t } = useTranslation();
  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("nav.sessions")}</h2>
      <SessionAnalysisPanel />
    </section>
  );
}
