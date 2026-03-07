import type { PropsWithChildren } from "react";

import { DisclosureCard } from "@/components/DisclosureCard";

interface DoctorDisclosureSectionProps extends PropsWithChildren {
  title: string;
}

export function DoctorDisclosureSection({
  title,
  children,
}: DoctorDisclosureSectionProps) {
  return (
    <DisclosureCard
      title={title}
      cardClassName="mt-8"
      bodyClassName="mt-3"
    >
      {children}
    </DisclosureCard>
  );
}
