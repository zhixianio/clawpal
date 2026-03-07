import type { PropsWithChildren } from "react";

import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

interface DisclosureCardProps extends PropsWithChildren {
  title: string;
  description?: string;
  defaultOpen?: boolean;
  cardClassName?: string;
  detailsClassName?: string;
  summaryClassName?: string;
  bodyClassName?: string;
}

export function DisclosureCard({
  title,
  description,
  defaultOpen = false,
  cardClassName,
  detailsClassName,
  summaryClassName,
  bodyClassName,
  children,
}: DisclosureCardProps) {
  return (
    <Card className={cardClassName}>
      <CardContent>
        <details
          open={defaultOpen ? true : undefined}
          className={cn(
            "rounded-lg border border-border/60 bg-muted/20 px-3 py-2",
            detailsClassName,
          )}
        >
          <summary
            className={cn(
              "cursor-pointer text-sm font-semibold text-foreground",
              summaryClassName,
            )}
          >
            {title}
          </summary>
          <div className={cn("mt-3 space-y-3", bodyClassName)}>
            {description ? (
              <p className="text-xs text-muted-foreground">{description}</p>
            ) : null}
            {children}
          </div>
        </details>
      </CardContent>
    </Card>
  );
}
