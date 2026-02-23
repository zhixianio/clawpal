import type { AgentOverview } from "./types";

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
