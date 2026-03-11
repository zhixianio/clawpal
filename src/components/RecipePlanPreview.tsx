import type { RecipePlan } from "@/lib/types";

function formatClaim(claim: RecipePlan["concreteClaims"][number]) {
  const details = [claim.id, claim.target, claim.path].filter(Boolean).join(" · ");
  return details ? `${claim.kind}: ${details}` : claim.kind;
}

export function RecipePlanPreview({ plan }: { plan: RecipePlan }) {
  return (
    <div className="mb-4 rounded-lg border border-border/70 bg-muted/20 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-medium">{plan.summary.recipeName}</div>
          <div className="text-xs text-muted-foreground">
            {plan.summary.executionKind} · {plan.summary.actionCount} action
            {plan.summary.actionCount === 1 ? "" : "s"}
            {plan.summary.skippedStepCount > 0
              ? ` · ${plan.summary.skippedStepCount} skipped`
              : ""}
          </div>
        </div>
        <div className="text-right">
          <div className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
            Execution Digest
          </div>
          <div className="font-mono text-xs">{plan.executionSpecDigest}</div>
        </div>
      </div>

      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <div>
          <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
            Capabilities
          </div>
          <div className="mt-2 flex flex-wrap gap-2">
            {plan.usedCapabilities.map((capability) => (
              <span
                key={capability}
                className="rounded-full bg-background px-2.5 py-1 font-mono text-xs"
              >
                {capability}
              </span>
            ))}
          </div>
        </div>

        <div>
          <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
            Resource Claims
          </div>
          <ul className="mt-2 space-y-1 text-sm text-muted-foreground">
            {plan.concreteClaims.map((claim, index) => (
              <li key={`${claim.kind}-${claim.id ?? claim.path ?? index}`}>
                {formatClaim(claim)}
              </li>
            ))}
          </ul>
        </div>
      </div>

      {plan.warnings.length > 0 && (
        <div className="mt-4 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-950">
          {plan.warnings.map((warning) => (
            <div key={warning}>{warning}</div>
          ))}
        </div>
      )}
    </div>
  );
}
