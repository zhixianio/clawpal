import type { PrecheckIssue, RecipePlan } from "@/lib/types";

type CookRouteContext = {
  instanceId: string;
  instanceLabel?: string | null;
  isRemote: boolean;
  isDocker: boolean;
};

export type CookRouteSummary = {
  kind: "local" | "docker" | "ssh";
  targetLabel: string;
};

export type CookAuthProfileScope = {
  requiredProfileIds: string[];
  autoPrepareProfileIds: string[];
};

type BindingEntry = {
  agentId?: string;
  match?: {
    channel?: string;
    peer?: {
      id?: string;
      kind?: string;
    };
  };
};

type ActionRecord = {
  kind?: unknown;
  args?: unknown;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function readPath(root: unknown, path: string[]): unknown {
  let current = root;
  for (const segment of path) {
    if (!isRecord(current)) {
      return undefined;
    }
    current = current[segment];
  }
  return current;
}

function collectPatchLeafWarnings(
  patch: unknown,
  currentConfig: unknown,
  path: string[],
  warnings: string[],
) {
  if (!isRecord(patch)) {
    const currentValue = readPath(currentConfig, path);
    if (typeof currentValue === "undefined") {
      return;
    }
    if (JSON.stringify(currentValue) === JSON.stringify(patch)) {
      return;
    }
    warnings.push(
      `Config path ${path.join(".")} will overwrite existing value.`,
    );
    return;
  }

  for (const [key, value] of Object.entries(patch)) {
    collectPatchLeafWarnings(value, currentConfig, [...path, key], warnings);
  }
}

function collectBindingWarnings(actions: ActionRecord[], config: unknown): string[] {
  const bindings = Array.isArray((config as { bindings?: unknown })?.bindings)
    ? ((config as { bindings: unknown[] }).bindings as BindingEntry[])
    : [];

  const warnings: string[] = [];
  for (const action of actions) {
    if (action.kind !== "bind_channel" || !isRecord(action.args)) {
      continue;
    }

    const channelType = typeof action.args.channelType === "string" ? action.args.channelType : null;
    const peerId = typeof action.args.peerId === "string" ? action.args.peerId : null;
    const agentId = typeof action.args.agentId === "string" ? action.args.agentId : null;
    if (!channelType || !peerId || !agentId) {
      continue;
    }

    const existing = bindings.find(
      (binding) =>
        binding.match?.channel === channelType &&
        binding.match?.peer?.kind === "channel" &&
        binding.match?.peer?.id === peerId,
    );
    if (!existing) {
      continue;
    }
    if (existing.agentId === agentId) {
      continue;
    }

    warnings.push(
      `Channel ${channelType}/${peerId} will be rebound from ${existing.agentId ?? "unknown"} to ${agentId}.`,
    );
  }

  return warnings;
}

function collectConfigPatchWarnings(actions: ActionRecord[], config: unknown): string[] {
  const warnings: string[] = [];
  for (const action of actions) {
    if (action.kind !== "config_patch" || !isRecord(action.args)) {
      continue;
    }

    collectPatchLeafWarnings(action.args.patch, config, [], warnings);
  }
  return warnings;
}

function normalizeRouteTarget(context: CookRouteContext): string {
  if (typeof context.instanceLabel === "string" && context.instanceLabel.trim().length > 0) {
    return context.instanceLabel.trim();
  }
  if (context.isRemote && context.instanceId.startsWith("ssh:")) {
    return context.instanceId.slice(4);
  }
  if (context.isDocker && context.instanceId.startsWith("docker:")) {
    return context.instanceId.slice(7);
  }
  return context.instanceId;
}

export function buildCookRouteSummary(context: CookRouteContext): CookRouteSummary {
  return {
    kind: context.isRemote ? "ssh" : context.isDocker ? "docker" : "local",
    targetLabel: normalizeRouteTarget(context),
  };
}

function readStringArg(args: unknown, key: string): string | null {
  if (!isRecord(args)) {
    return null;
  }
  const value = args[key];
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function extractProfileIdFromIssue(issue: PrecheckIssue): string | null {
  const match = issue.message.match(/Profile '([^']+)'/);
  return match?.[1] ?? null;
}

export function buildCookAuthProfileScope(plan: RecipePlan): CookAuthProfileScope {
  const requiredProfileIds = new Set<string>();
  const autoPrepareProfileIds = new Set<string>();

  for (const claim of plan.concreteClaims) {
    if (claim.kind === "modelProfile" && typeof claim.id === "string" && claim.id.trim().length > 0) {
      requiredProfileIds.add(claim.id.trim());
    }
  }

  for (const action of plan.executionSpec.actions.filter(isRecord) as ActionRecord[]) {
    const kind = typeof action.kind === "string" ? action.kind : null;
    if (!kind) {
      continue;
    }

    if (kind === "ensure_model_profile") {
      const profileId = readStringArg(action.args, "profileId");
      if (!profileId) {
        continue;
      }
      requiredProfileIds.add(profileId);
      autoPrepareProfileIds.add(profileId);
      continue;
    }

    if (kind === "set_agent_model") {
      const profileId = readStringArg(action.args, "profileId");
      if (!profileId) {
        continue;
      }
      requiredProfileIds.add(profileId);
      const ensureProfile = isRecord(action.args) ? action.args.ensureProfile : undefined;
      if (ensureProfile !== false) {
        autoPrepareProfileIds.add(profileId);
      }
      continue;
    }

    if (kind === "create_agent") {
      const profileId = readStringArg(action.args, "modelProfileId");
      if (profileId) {
        requiredProfileIds.add(profileId);
      }
    }
  }

  return {
    requiredProfileIds: Array.from(requiredProfileIds),
    autoPrepareProfileIds: Array.from(autoPrepareProfileIds),
  };
}

export function filterCookAuthIssues(
  issues: PrecheckIssue[],
  scope: CookAuthProfileScope,
): PrecheckIssue[] {
  if (scope.requiredProfileIds.length === 0) {
    return [];
  }

  const required = new Set(scope.requiredProfileIds);
  const autoPrepare = new Set(scope.autoPrepareProfileIds);

  return issues.filter((issue) => {
    const profileId = extractProfileIdFromIssue(issue);
    if (!profileId) {
      return true;
    }
    if (!required.has(profileId)) {
      return false;
    }
    if (autoPrepare.has(profileId) && issue.code === "AUTH_CREDENTIAL_UNRESOLVED") {
      return false;
    }
    return true;
  });
}

export function buildCookContextWarnings(
  plan: RecipePlan,
  rawConfig: string | null | undefined,
): string[] {
  if (!rawConfig) {
    return [];
  }

  let parsedConfig: unknown;
  try {
    parsedConfig = JSON.parse(rawConfig);
  } catch {
    return [];
  }

  const actions = plan.executionSpec.actions.filter(isRecord) as ActionRecord[];
  return [
    ...collectBindingWarnings(actions, parsedConfig),
    ...collectConfigPatchWarnings(actions, parsedConfig),
  ];
}

export function hasBlockingAuthIssues(issues: PrecheckIssue[]): boolean {
  return issues.some((issue) => issue.severity === "error");
}
