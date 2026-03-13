import { api } from "./api";
import { explainAndBuildGuidanceError } from "./guidance";
import type { ModelProfile } from "./types";
import { profileToModelValue } from "./model-value";

export interface ActionContext {
  instanceId: string;
  isRemote: boolean;
}

async function callWithLobsterGuidance<T>(
  method: string,
  ctx: ActionContext | undefined,
  fn: () => Promise<T>,
): Promise<T> {
  try {
    return await fn();
  } catch (error) {
    throw await explainAndBuildGuidanceError({
      method,
      instanceId: ctx?.instanceId || "local",
      transport: ctx?.isRemote ? "remote_ssh" : "local",
      rawError: error,
      emitEvent: true,
    });
  }
}

/** Resolve a profile ID to a "provider/model" string from the local ClawPal profile hub. */
async function resolveProfileToModelValue(
  profileId: string | undefined,
  ctx?: ActionContext,
): Promise<string | undefined> {
  if (!profileId || profileId === "__default__") return undefined;
  const profiles: ModelProfile[] = await callWithLobsterGuidance(
    "resolveProfileToModelValue",
    ctx,
    () => api.listModelProfiles(),
  );
  const profile = profiles.find((p) => p.id === profileId);
  if (!profile) return profileId; // fallback: use raw string
  return profileToModelValue(profile);
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

function noopCommands(): Promise<[string, string[]][]> {
  return Promise.resolve([]);
}

async function unsupportedCommands(action: string): Promise<[string, string[]][]> {
  throw new Error(`${action} is documented but not supported by the local Recipe command preview`);
}

function stringList(value: unknown): string[] {
  if (typeof value === "string") {
    return value
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }
  if (Array.isArray(value)) {
    return value
      .filter((item): item is string => typeof item === "string")
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return [];
}

const registry: Record<string, ActionDef> = {
  create_agent: {
    toCommands: async (args, ctx) => {
      const modelValue = await resolveProfileToModelValue(
        args.modelProfileId as string | undefined,
        ctx,
      );
      let workspace: string | undefined;
      if (ctx) {
        try {
          const rawConfig = await callWithLobsterGuidance(
            "readRawConfig",
            ctx,
            () => (ctx.isRemote ? api.remoteReadRawConfig(ctx.instanceId) : api.readRawConfig()),
          );
          const cfg = JSON.parse(rawConfig);
          workspace = cfg?.agents?.defaults?.workspace ?? cfg?.agents?.default?.workspace;
        } catch {
          // ignore and fall back to agent overview
        }
        if (!workspace) {
          try {
            const agents = await callWithLobsterGuidance(
              "listAgentsOverview",
              ctx,
              () => (ctx.isRemote
                ? api.remoteListAgentsOverview(ctx.instanceId)
                : api.listAgentsOverview()),
            );
            workspace = agents.find((agent) => agent.workspace)?.workspace ?? undefined;
          } catch {
            // ignore and surface a precise error below if still unresolved
          }
        }
      }
      const cmd: string[] = ["openclaw", "agents", "add", args.agentId as string, "--non-interactive"];
      if (workspace) cmd.push("--workspace", workspace);
      if (modelValue) cmd.push("--model", modelValue);
      return [[`Create agent: ${args.agentId}`, cmd]];
    },
    describe: (args) => {
      const model = args.modelProfileId as string | undefined;
      const modelLabel = !model || model === "__default__" ? "default model" : model;
      return `Create agent "${args.agentId}" (${modelLabel})`;
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
      const name = typeof args.name === "string" && args.name.trim().length > 0
        ? args.name
        : undefined;
      const emoji = typeof args.emoji === "string" && args.emoji.trim().length > 0
        ? ` ${args.emoji}`
        : "";
      if (name) {
        return `Set identity: ${name}${emoji}`;
      }
      if (args.agentId) {
        return `Update persona: ${args.agentId}`;
      }
      return "Update agent identity";
    },
  },
  set_agent_identity: {
    toCommands: async (args) => {
      const fromIdentity = args.fromIdentity === true;
      const agentId = typeof args.agentId === "string" ? args.agentId : undefined;
      const workspace = typeof args.workspace === "string" ? args.workspace : undefined;
      const command = ["openclaw", "agents", "set-identity"];
      if (agentId) command.push("--agent", agentId);
      if (workspace) command.push("--workspace", workspace);
      if (fromIdentity) command.push("--from-identity");
      if (typeof args.name === "string" && args.name.trim()) command.push("--name", args.name);
      if (typeof args.theme === "string" && args.theme.trim()) command.push("--theme", args.theme);
      if (typeof args.emoji === "string" && args.emoji.trim()) command.push("--emoji", args.emoji);
      if (typeof args.avatar === "string" && args.avatar.trim()) command.push("--avatar", args.avatar);
      return [[agentId ? `Set identity: ${agentId}` : "Set identity from workspace", command]];
    },
    describe: (args) => {
      if (typeof args.agentId === "string" && args.agentId.trim()) {
        return `Update identity fields for agent "${args.agentId}"`;
      }
      if (args.fromIdentity) {
        return "Load identity fields from workspace";
      }
      return "Update agent identity";
    },
  },
  delete_agent: {
    toCommands: async (args) => {
      const command = ["openclaw", "agents", "delete", String(args.agentId)];
      if (args.force === true) command.push("--force");
      return [[`Delete agent: ${args.agentId}`, command]];
    },
    describe: (args) => `Delete agent "${args.agentId}"`,
  },
  bind_agent: {
    toCommands: async (args) => [[
      `Bind ${args.binding} → ${args.agentId}`,
      ["openclaw", "agents", "bind", "--agent", String(args.agentId), "--bind", String(args.binding)],
    ]],
    describe: (args) => `Bind ${args.binding} to agent "${args.agentId}"`,
  },
  unbind_agent: {
    toCommands: async (args) => {
      const command = ["openclaw", "agents", "unbind", "--agent", String(args.agentId)];
      if (args.all === true) command.push("--all");
      else if (args.binding) command.push("--bind", String(args.binding));
      return [[`Unbind ${args.agentId}`, command]];
    },
    describe: (args) => args.all === true
      ? `Remove all bindings from agent "${args.agentId}"`
      : `Remove binding ${args.binding} from agent "${args.agentId}"`,
  },
  bind_channel: {
    toCommands: async (args, ctx) => {
      const agentId = args.agentId as string;
      const channelType = args.channelType as string;
      const peerId = args.peerId as string;
      // Read current bindings, add new binding, set full array
      const bindings: unknown[] = await callWithLobsterGuidance(
        "listBindings",
        ctx,
        () => (ctx?.isRemote
          ? api.remoteListBindings(ctx.instanceId)
          : api.listBindings()),
      );
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
  unbind_channel: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Remove binding for ${args.channelType} channel ${args.peerId}`,
  },
  set_agent_model: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Set agent "${args.agentId}" to model profile "${args.profileId}"`,
  },
  set_agent_persona: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Apply persona to agent "${args.agentId}"`,
  },
  clear_agent_persona: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Clear persona for agent "${args.agentId}"`,
  },
  set_channel_persona: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Set persona for ${args.channelType} channel "${args.peerId}"`,
  },
  clear_channel_persona: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Clear persona for ${args.channelType} channel "${args.peerId}"`,
  },
  upsert_markdown_document: {
    toCommands: async () => noopCommands(),
    describe: (args) => {
      const target = args.target as Record<string, unknown> | undefined;
      const scope = typeof target?.scope === "string" ? target.scope : "document";
      const path = typeof target?.path === "string" ? target.path : "";
      return `Update ${scope} document ${path}`.trim();
    },
  },
  delete_markdown_document: {
    toCommands: async () => noopCommands(),
    describe: (args) => {
      const target = args.target as Record<string, unknown> | undefined;
      const scope = typeof target?.scope === "string" ? target.scope : "document";
      const path = typeof target?.path === "string" ? target.path : "";
      return `Delete ${scope} document ${path}`.trim();
    },
  },
  ensure_model_profile: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Prepare model access for profile "${args.profileId}"`,
  },
  delete_model_profile: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Remove model profile "${args.profileId}"`,
  },
  ensure_provider_auth: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Prepare provider auth for "${args.provider}"`,
  },
  delete_provider_auth: {
    toCommands: async () => noopCommands(),
    describe: (args) => `Remove provider auth "${args.authRef}"`,
  },
  list_agents: {
    toCommands: async () => [["List agents", ["openclaw", "agents", "list", "--json"]]],
    describe: () => "List agents",
  },
  list_agent_bindings: {
    toCommands: async () => [["List agent bindings", ["openclaw", "agents", "bindings"]]],
    describe: () => "List agent bindings",
  },
  show_config_file: {
    toCommands: async () => [["Show config file", ["openclaw", "config", "file"]]],
    describe: () => "Show the active config file path",
  },
  get_config_value: {
    toCommands: async (args) => [[
      `Get config value: ${args.path}`,
      ["openclaw", "config", "get", String(args.path)],
    ]],
    describe: (args) => `Read config value ${args.path}`,
  },
  set_config_value: {
    toCommands: async (args) => {
      const value = args.value;
      const strictJson = args.strictJson === true || typeof value !== "string";
      const serialized = strictJson ? JSON.stringify(value) : String(value ?? "");
      const command = ["openclaw", "config", "set", String(args.path), serialized];
      if (strictJson) command.push("--strict-json");
      return [[`Set config value: ${args.path}`, command]];
    },
    describe: (args) => `Set config value ${args.path}`,
  },
  unset_config_value: {
    toCommands: async (args) => [[
      `Unset config value: ${args.path}`,
      ["openclaw", "config", "unset", String(args.path)],
    ]],
    describe: (args) => `Unset config value ${args.path}`,
  },
  validate_config: {
    toCommands: async (args) => {
      const command = ["openclaw", "config", "validate"];
      if (args.jsonOutput === true) command.push("--json");
      return [["Validate config", command]];
    },
    describe: () => "Validate the active config",
  },
  models_status: {
    toCommands: async (args) => {
      const command = ["openclaw", "models", "status"];
      if (args.jsonOutput === true) command.push("--json");
      if (args.plain === true) command.push("--plain");
      if (args.check === true) command.push("--check");
      if (args.probe === true) command.push("--probe");
      if (typeof args.probeProvider === "string" && args.probeProvider.trim()) {
        command.push("--probe-provider", args.probeProvider);
      }
      for (const profile of stringList(args.probeProfile)) {
        command.push("--probe-profile", profile);
      }
      if (typeof args.probeTimeoutMs === "string" && args.probeTimeoutMs.trim()) {
        command.push("--probe-timeout", args.probeTimeoutMs);
      }
      if (typeof args.probeConcurrency === "string" && args.probeConcurrency.trim()) {
        command.push("--probe-concurrency", args.probeConcurrency);
      }
      if (typeof args.probeMaxTokens === "string" && args.probeMaxTokens.trim()) {
        command.push("--probe-max-tokens", args.probeMaxTokens);
      }
      if (typeof args.agentId === "string" && args.agentId.trim()) {
        command.push("--agent", args.agentId);
      }
      return [["Inspect model status", command]];
    },
    describe: () => "Inspect model status",
  },
  list_models: {
    toCommands: async () => [["List models", ["openclaw", "models", "list"]]],
    describe: () => "List available models",
  },
  set_default_model: {
    toCommands: async (args) => [[
      `Set default model: ${args.modelOrAlias}`,
      ["openclaw", "models", "set", String(args.modelOrAlias)],
    ]],
    describe: (args) => `Set the default model to ${args.modelOrAlias}`,
  },
  scan_models: {
    toCommands: async () => [["Scan models", ["openclaw", "models", "scan"]]],
    describe: () => "Scan model availability",
  },
  list_model_aliases: {
    toCommands: async () => [["List model aliases", ["openclaw", "models", "aliases", "list"]]],
    describe: () => "List model aliases",
  },
  list_model_fallbacks: {
    toCommands: async () => [["List model fallbacks", ["openclaw", "models", "fallbacks", "list"]]],
    describe: () => "List model fallbacks",
  },
  add_model_auth_profile: {
    toCommands: async () => unsupportedCommands("add_model_auth_profile"),
    describe: () => "Add a provider auth profile",
  },
  login_model_auth: {
    toCommands: async () => unsupportedCommands("login_model_auth"),
    describe: () => "Run a provider auth login flow",
  },
  setup_model_auth_token: {
    toCommands: async () => unsupportedCommands("setup_model_auth_token"),
    describe: () => "Prompt for a setup token",
  },
  paste_model_auth_token: {
    toCommands: async () => unsupportedCommands("paste_model_auth_token"),
    describe: () => "Paste a model auth token",
  },
  list_channels: {
    toCommands: async (args) => {
      const command = ["openclaw", "channels", "list"];
      if (args.noUsage === true) command.push("--no-usage");
      return [["List channels", command]];
    },
    describe: () => "List configured channels",
  },
  channels_status: {
    toCommands: async () => [["Inspect channel status", ["openclaw", "channels", "status"]]],
    describe: () => "Inspect channel status",
  },
  read_channel_logs: {
    toCommands: async () => unsupportedCommands("read_channel_logs"),
    describe: () => "Read channel logs",
  },
  add_channel_account: {
    toCommands: async () => unsupportedCommands("add_channel_account"),
    describe: () => "Add a channel account",
  },
  remove_channel_account: {
    toCommands: async () => unsupportedCommands("remove_channel_account"),
    describe: () => "Remove a channel account",
  },
  login_channel_account: {
    toCommands: async () => unsupportedCommands("login_channel_account"),
    describe: () => "Run a channel login flow",
  },
  logout_channel_account: {
    toCommands: async () => unsupportedCommands("logout_channel_account"),
    describe: () => "Run a channel logout flow",
  },
  inspect_channel_capabilities: {
    toCommands: async (args) => {
      const command = ["openclaw", "channels", "capabilities"];
      if (typeof args.channel === "string" && args.channel.trim()) {
        command.push("--channel", args.channel);
      }
      if (typeof args.target === "string" && args.target.trim()) {
        command.push("--target", args.target);
      }
      return [["Inspect channel capabilities", command]];
    },
    describe: () => "Inspect channel capabilities",
  },
  resolve_channel_targets: {
    toCommands: async (args) => {
      const command = ["openclaw", "channels", "resolve", "--channel", String(args.channel)];
      if (typeof args.kind === "string" && args.kind.trim()) {
        command.push("--kind", args.kind);
      }
      command.push(...stringList(args.terms));
      return [["Resolve channel targets", command]];
    },
    describe: (args) => `Resolve targets in ${args.channel}`,
  },
  reload_secrets: {
    toCommands: async () => [["Reload secrets", ["openclaw", "secrets", "reload"]]],
    describe: () => "Reload runtime secrets",
  },
  audit_secrets: {
    toCommands: async (args) => {
      const command = ["openclaw", "secrets", "audit"];
      if (args.check === true) command.push("--check");
      return [["Audit secrets", command]];
    },
    describe: () => "Audit secret references",
  },
  configure_secrets: {
    toCommands: async () => unsupportedCommands("configure_secrets"),
    describe: () => "Run the interactive secret configuration flow",
  },
  apply_secrets_plan: {
    toCommands: async (args) => {
      const command = ["openclaw", "secrets", "apply", "--from", String(args.fromPath)];
      if (args.dryRun === true) command.push("--dry-run");
      if (args.jsonOutput === true) command.push("--json");
      return [[`Apply secrets plan: ${args.fromPath}`, command]];
    },
    describe: (args) => `Apply secrets plan from ${args.fromPath}`,
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
