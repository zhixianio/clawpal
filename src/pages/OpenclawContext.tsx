import { useTranslation } from "react-i18next";

import { DoctorDisclosureSection } from "@/components/DoctorDisclosureSection";
import { SessionAnalysisPanel } from "@/components/SessionAnalysisPanel";
import { BackupsPanel } from "@/components/BackupsPanel";

export function OpenclawContext() {
  const { t } = useTranslation();

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("nav.context")}</h2>
      <DoctorDisclosureSection title={t("doctor.sessions")}>
        <SessionAnalysisPanel />
      </DoctorDisclosureSection>
      <DoctorDisclosureSection title={t("doctor.backups")}>
        <BackupsPanel />
      </DoctorDisclosureSection>
    </section>
  );
}
