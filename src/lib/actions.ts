import { api } from "./api";
import type { ModelProfile } from "./types";

export interface ActionContext {
  instanceId: string;
  isRemote: boolean;
}

/** Resolve a profile ID to a "provider/model" string by loading profiles from local or remote. */
async function resolveProfileToModelValue(
  profileId: string | undefined,
  ctx?: ActionContext,
): Promise<string | undefined> {
  if (!profileId || profileId === "__default__") return undefined;
  const profiles: ModelProfile[] = ctx?.isRemote
    ? await api.remoteListModelProfiles(ctx.instanceId)
    : await api.listModelProfiles();
  const profile = profiles.find((p) => p.id === profileId);
  if (!profile) return profileId; // fallback: use raw string
  return profile.model.includes("/")
    ? profile.model
    : `${profile.provider}/${profile.model}`;
}

export interface ActionDef {
  /** Returns (label, command[]) tuples to queue instead of executing directly. */
  toCommands: (args: Record<string, unknown>, ctx?: ActionContext) => Promise<[string, string[]][]>;
  describe: (args: Record<string, unknown>) => string;
}

function renderArgs(
  args: Record<string, unknown>,
  params: Record<string, string>,
): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) {
    if (typeof value === "string") {
      const singleMatch = value.match(/^\{\{(\w+)\}\}$/);
      if (singleMatch) {
        const paramValue = params[singleMatch[1]] ?? "";
        if (paramValue === "true") result[key] = true;
        else if (paramValue === "false") result[key] = false;
        else result[key] = paramValue;
      } else {
        let rendered = value;
        for (const [paramId, paramValue] of Object.entries(params)) {
          rendered = rendered.split(`{{${paramId}}}`).join(paramValue);
        }
        result[key] = rendered;
      }
    } else {
      result[key] = value;
    }
  }
  return result;
}

const registry: Record<string, ActionDef> = {
  create_agent: {
    toCommands: async (args, ctx) => {
      const modelValue = await resolveProfileToModelValue(
        args.modelProfileId as string | undefined,
        ctx,
      );
      const cmd: string[] = ["openclaw", "agents", "add", args.agentId as string, "--non-interactive"];
      if (modelValue) cmd.push("--model", modelValue);
      if (args.independent) cmd.push("--workspace", args.agentId as string);
      return [[`Create agent: ${args.agentId}`, cmd]];
    },
    describe: (args) => {
      const model = args.modelProfileId as string | undefined;
      const modelLabel = !model || model === "__default__" ? "default model" : model;
      return `Create ${args.independent ? "independent " : ""}agent "${args.agentId}" (${modelLabel})`;
    },
  },
  setup_identity: {
    toCommands: async (args) => {
      // Identity setup is a filesystem operation, not a config set.
      // Queue as a config set for the agent's display name/emoji via the workspace.
      // For now, skip — identity is set during agent add or via CLI manually.
      return [];
    },
    describe: (args) => {
      const emoji = args.emoji ? ` ${args.emoji}` : "";
      return `Set identity: ${args.name}${emoji}`;
    },
  },
  bind_channel: {
    toCommands: async (args, ctx) => {
      const agentId = args.agentId as string;
      const channelType = args.channelType as string;
      const peerId = args.peerId as string;
      // Read current bindings, add new binding, set full array
      const bindings: unknown[] = ctx?.isRemote
        ? await api.remoteListBindings(ctx.instanceId)
        : await api.listBindings();
      // Remove existing binding for same channel+peer
      const filtered = (bindings as Array<Record<string, unknown>>).filter((b) => {
        const m = b.match as Record<string, unknown> | undefined;
        if (!m) return true;
        const ch = m.channel;
        const peer = m.peer as Record<string, unknown> | undefined;
        return !(ch === channelType && peer?.id === peerId);
      });
      filtered.push({
        agentId,
        match: { channel: channelType, peer: { kind: "channel", id: peerId } },
      });
      return [[
        `Bind ${channelType}:${peerId} → ${agentId}`,
        ["openclaw", "config", "set", "bindings", JSON.stringify(filtered), "--json"],
      ]];
    },
    describe: (args) =>
      `Bind ${args.channelType} channel → agent "${args.agentId}"`,
  },
  config_patch: {
    toCommands: async (args) => {
      const patchTemplate = args.patchTemplate as string;
      const params = args.params as Record<string, string>;
      // Render the template
      let rendered = patchTemplate;
      for (const [key, value] of Object.entries(params)) {
        rendered = rendered.split(`{{${key}}}`).join(value);
      }
      // Parse as JSON and walk top-level keys to produce config set commands
      try {
        const patch = JSON.parse(rendered);
        const commands: [string, string[]][] = [];
        const walk = (obj: Record<string, unknown>, path: string) => {
          for (const [key, value] of Object.entries(obj)) {
            const fullPath = path ? `${path}.${key}` : key;
            if (value && typeof value === "object" && !Array.isArray(value)) {
              walk(value as Record<string, unknown>, fullPath);
            } else {
              const jsonVal = JSON.stringify(value);
              commands.push([
                `Set ${fullPath}`,
                ["openclaw", "config", "set", fullPath, jsonVal, "--json"],
              ]);
            }
          }
        };
        walk(patch, "");
        return commands;
      } catch {
        // Fallback: treat entire patch as a single set
        return [[
          "Apply config patch",
          ["openclaw", "config", "set", ".", rendered, "--json"],
        ]];
      }
    },
    describe: () => "",
  },
  set_global_model: {
    toCommands: async (args, ctx) => {
      const modelValue = await resolveProfileToModelValue(
        args.profileId as string | undefined,
        ctx,
      ) ?? null;
      if (modelValue) {
        return [[
          `Set global model: ${modelValue}`,
          ["openclaw", "config", "set", "agents.defaults.model.primary", modelValue],
        ]];
      }
      return [[
        "Clear global model",
        ["openclaw", "config", "unset", "agents.defaults.model.primary"],
      ]];
    },
    describe: (args) => `Set default model to ${args.profileId}`,
  },
};

export function getAction(actionType: string): ActionDef | undefined {
  return registry[actionType];
}

export interface ResolvedStep {
  index: number;
  action: string;
  label: string;
  args: Record<string, unknown>;
  description: string;
  skippable: boolean;
}

export function resolveSteps(
  steps: { action: string; label: string; args: Record<string, unknown> }[],
  params: Record<string, string>,
): ResolvedStep[] {
  return steps.map((step, index) => {
    const resolved = renderArgs(step.args, params);
    if (step.action === "config_patch") {
      resolved.params = params;
    }
    const skippable = Object.values(step.args).some((origValue) => {
      if (typeof origValue !== "string") return false;
      const matches = origValue.match(/\{\{(\w+)\}\}/g);
      if (!matches) return false;
      return matches.some((m) => {
        const paramId = m.slice(2, -2);
        const val = params[paramId];
        return val !== undefined && val.trim() === "";
      });
    });
    const actionDef = getAction(step.action);
    const description = actionDef?.describe(resolved) || step.label;
    return {
      index,
      action: step.action,
      label: step.label,
      args: resolved,
      description: description || step.label,
      skippable,
    };
  });
}

/** Convert a resolved step into CLI commands for the queue. */
export async function stepToCommands(
  step: ResolvedStep,
  ctx?: ActionContext,
): Promise<[string, string[]][]> {
  const actionDef = getAction(step.action);
  if (!actionDef) {
    throw new Error(`Unknown action type: ${step.action}`);
  }
  return actionDef.toCommands(step.args, ctx);
}
