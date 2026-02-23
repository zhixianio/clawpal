import type { AgentOverview, ModelProfile } from "./types";

/**
 * Find the profile ID whose model value matches the given raw model string.
 * Handles normalization (case-insensitive, with or without provider/ prefix).
 * Returns the matching profile's `id`, or `null` if no profile matches.
 */
export function findProfileIdByModelValue(
  modelValue: string | null | undefined,
  profiles: ModelProfile[],
): string | null {
  if (!modelValue) return null;
  const normalized = modelValue.toLowerCase();
  for (const p of profiles) {
    const profileVal = p.model.includes("/") ? p.model : `${p.provider}/${p.model}`;
    if (profileVal.toLowerCase() === normalized || p.model.toLowerCase() === normalized) {
      return p.id;
    }
  }
  return null;
}

export interface AgentGroup {
  identity: string;
  emoji?: string;
  agents: AgentOverview[];
}

export function groupAgents(agents: AgentOverview[]): AgentGroup[] {
  const map = new Map<string, AgentGroup>();
  for (const a of agents) {
    // Group by workspace path (shared identity), fallback to agent id
    const key = a.workspace || a.id;
    if (!map.has(key)) {
      map.set(key, {
        identity: a.name || a.id,
        emoji: a.emoji,
        agents: [],
      });
    }
    map.get(key)!.agents.push(a);
  }
  return Array.from(map.values());
}
