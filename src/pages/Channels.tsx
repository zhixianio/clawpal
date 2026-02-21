import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AgentOverview, Binding, ChannelNode, DiscordGuildChannel, ModelProfile } from "../lib/types";
import { useApi } from "@/lib/use-api";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CreateAgentDialog, type CreateAgentResult } from "@/components/CreateAgentDialog";

interface AgentGroup {
  identity: string;
  emoji?: string;
  agents: AgentOverview[];
}

const PLATFORM_LABELS: Record<string, string> = {
  discord: "Discord",
  telegram: "Telegram",
  feishu: "Feishu",
  qbot: "QBot",
};

function groupAgents(agents: AgentOverview[]): AgentGroup[] {
  const map = new Map<string, AgentGroup>();
  for (const a of agents) {
    const key = a.workspace || a.id;
    if (!map.has(key)) {
      map.set(key, { identity: a.name || a.id, emoji: a.emoji, agents: [] });
    }
    map.get(key)!.agents.push(a);
  }
  return Array.from(map.values());
}

function extractPlatform(path: string): string | null {
  const parts = path.split(".");
  if (parts.length >= 2 && parts[0] === "channels") return parts[1];
  return null;
}

function extractPeerId(path: string): string {
  return path.split(".").pop() || path;
}

export function Channels({
  showToast,
}: {
  showToast?: (message: string, type?: "success" | "error") => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [agents, setAgents] = useState<AgentOverview[]>([]);
  const [bindings, setBindings] = useState<Binding[]>([]);
  const [modelProfiles, setModelProfiles] = useState<ModelProfile[]>([]);
  const [channelNodes, setChannelNodes] = useState<ChannelNode[]>([]);
  const [discordChannels, setDiscordChannels] = useState<DiscordGuildChannel[] | null>(null);
  const [refreshing, setRefreshing] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);

  // Create agent dialog
  const [showCreateAgent, setShowCreateAgent] = useState(false);
  const [pendingChannel, setPendingChannel] = useState<{
    platform: string;
    peerId: string;
    guildId?: string;
  } | null>(null);

  const refreshAgents = useCallback(() => {
    ua.listAgents().then(setAgents).catch((e) => console.error("Failed to load agents:", e));
  }, [ua]);

  const refreshBindings = useCallback(() => {
    ua.listBindings().then((b) => setBindings(b)).catch((e) => console.error("Failed to load bindings:", e));
  }, [ua]);

  const refreshChannelNodes = useCallback(() => {
    ua.listChannels().then(setChannelNodes).catch((e) => console.error("Failed to load channel nodes:", e));
  }, [ua]);

  const refreshDiscordCache = useCallback(() => {
    ua.listDiscordGuildChannels().then(setDiscordChannels).catch((e) => console.error("Failed to load Discord channels:", e));
  }, [ua]);

  useEffect(() => {
    refreshAgents();
    refreshBindings();
    ua.listModelProfiles().then((p) => setModelProfiles(p.filter((m) => m.enabled))).catch((e) => console.error("Failed to load model profiles:", e));
    refreshChannelNodes();
    refreshDiscordCache();
  }, [ua, refreshAgents, refreshBindings, refreshChannelNodes, refreshDiscordCache]);

  const handleRefreshDiscord = () => {
    setRefreshing("discord");
    ua.refreshDiscordGuildChannels()
      .then((channels) => {
        setDiscordChannels(channels);
        showToast?.(t('channels.discordRefreshed'), "success");
      })
      .catch((e) => showToast?.(String(e), "error"))
      .finally(() => setRefreshing(null));
  };

  const handleRefreshPlatform = (platform: string) => {
    setRefreshing(platform);
    ua.listChannels()
      .then((nodes) => {
        setChannelNodes(nodes);
        showToast?.(t('channels.platformRefreshed', { platform: PLATFORM_LABELS[platform] || platform }), "success");
      })
      .catch((e) => showToast?.(String(e), "error"))
      .finally(() => setRefreshing(null));
  };

  // Binding lookup: "platform:peerId" -> agentId
  const channelAgentMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const b of bindings) {
      if (b.match?.channel && b.match?.peer?.id) {
        map.set(`${b.match.channel}:${b.match.peer.id}`, b.agentId);
      }
    }
    return map;
  }, [bindings]);

  // Discord channels grouped by guild
  const discordGuilds = useMemo(() => {
    const map = new Map<string, { guildName: string; channels: DiscordGuildChannel[] }>();
    for (const ch of discordChannels || []) {
      if (!map.has(ch.guildId)) {
        map.set(ch.guildId, { guildName: ch.guildName, channels: [] });
      }
      map.get(ch.guildId)!.channels.push(ch);
    }
    return Array.from(map.entries());
  }, [discordChannels]);

  // Non-Discord channel nodes grouped by platform, filtered to leaf-level
  const otherPlatforms = useMemo(() => {
    const map = new Map<string, ChannelNode[]>();
    for (const node of channelNodes) {
      const platform = extractPlatform(node.path);
      if (!platform || platform === "discord") continue;
      if (node.channelType === "platform") continue;
      if (!map.has(platform)) map.set(platform, []);
      map.get(platform)!.push(node);
    }
    return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [channelNodes]);

  const agentGroups = useMemo(() => groupAgents(agents), [agents]);

  const handleAssign = async (platform: string, peerId: string, agentId: string) => {
    if (agentId === "__new__") {
      setPendingChannel({ platform, peerId });
      setShowCreateAgent(true);
      return;
    }
    const key = `${platform}:${peerId}`;
    setSaving(key);
    try {
      const resolvedAgent = agentId === "__default__" ? null : agentId;
      // Build new bindings array: remove existing binding for this peer, optionally add new one
      const newBindings = bindings.filter((b) => {
        const ch = b.match?.channel;
        const pid = b.match?.peer?.id;
        return !(ch === platform && pid === peerId);
      });
      if (resolvedAgent) {
        newBindings.push({
          agentId: resolvedAgent,
          match: { channel: platform, peer: { kind: "channel", id: peerId } },
        });
      }
      await ua.queueCommand(
        resolvedAgent
          ? `Bind ${platform}:${peerId} → ${resolvedAgent}`
          : `Unbind ${platform}:${peerId}`,
        ["openclaw", "config", "set", "bindings", JSON.stringify(newBindings), "--json"],
      );
      refreshBindings();
    } catch (e) {
      showToast?.(String(e), "error");
    } finally {
      setSaving(null);
    }
  };

  function agentDisplayLabel(agentId: string): string {
    const a = agents.find((ag) => ag.id === agentId);
    if (!a) return agentId;
    const name = a.name || a.id;
    const emoji = a.emoji ? `${a.emoji} ` : "";
    const model = a.model ? ` (${a.model})` : "";
    return `${emoji}${name}: ${a.id}${model}`;
  }

  const renderAgentSelect = (platform: string, peerId: string) => {
    const key = `${platform}:${peerId}`;
    const currentAgent = channelAgentMap.get(key);
    return (
      <Select
        value={currentAgent || "__default__"}
        onValueChange={(val) => handleAssign(platform, peerId, val)}
        disabled={saving === key}
      >
        <SelectTrigger size="sm" className="text-xs">
          <SelectValue>
            {currentAgent ? agentDisplayLabel(currentAgent) : t('channels.mainDefault')}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__default__">
            <span className="text-muted-foreground">{t('channels.mainDefault')}</span>
          </SelectItem>
          {agentGroups.map((group, gi) => (
            <SelectGroup key={group.agents[0].workspace || group.agents[0].id}>
              {gi > 0 && <SelectSeparator />}
              <SelectLabel>
                {group.emoji ? `${group.emoji} ` : ""}{group.identity}
              </SelectLabel>
              {group.agents.map((a) => (
                <SelectItem key={a.id} value={a.id}>
                  <code className="text-xs">{a.id}</code>
                  <span className="text-muted-foreground ml-1.5 text-xs">
                    {a.model || t('channels.defaultModel')}
                  </span>
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
          <>
            <SelectSeparator />
            <SelectItem value="__new__">
              <span className="text-primary">{t('channels.newAgent')}</span>
            </SelectItem>
          </>
        </SelectContent>
      </Select>
    );
  };

  const hasDiscord = (discordChannels || []).length > 0;
  const hasOther = otherPlatforms.length > 0;

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('channels.title')}</h2>

      {!hasDiscord && !hasOther && (
        <p className="text-muted-foreground">
          {t('channels.noChannels')}
        </p>
      )}

      <div className="space-y-6">
        {/* Discord section */}
        <Card>
          <CardContent>
            <div className="flex items-center gap-2 mb-3">
              <strong className="text-lg">Discord</strong>
              <Button
                variant="outline"
                size="sm"
                className="ml-auto"
                onClick={handleRefreshDiscord}
                disabled={refreshing === "discord"}
              >
                {refreshing === "discord" ? t('channels.refreshing') : t('channels.refresh')}
              </Button>
            </div>

            {discordChannels === null ? (
              <p className="text-sm text-muted-foreground animate-pulse">{t('channels.loadingDiscord')}</p>
            ) : discordGuilds.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                {t('channels.noDiscordChannels')}
              </p>
            ) : (
              <div className="space-y-4">
                {discordGuilds.map(([guildId, { guildName, channels }]) => (
                  <div key={guildId}>
                    <div className="flex items-center gap-1.5 mb-2">
                      <span className="text-sm font-medium">{guildName}</span>
                      <Badge variant="secondary" className="text-[10px]">{guildId}</Badge>
                    </div>
                    <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-2">
                      {channels.map((ch) => (
                        <div key={ch.channelId} className="rounded-md border px-3 py-2">
                          <div className="text-sm font-medium">{ch.channelName}</div>
                          <div className="text-xs text-muted-foreground mt-0.5 mb-1.5">{ch.channelId}</div>
                          {renderAgentSelect("discord", ch.channelId)}
                        </div>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Other platform sections */}
        {otherPlatforms.map(([platform, nodes]) => (
          <Card key={platform}>
            <CardContent>
              <div className="flex items-center gap-2 mb-3">
                <strong className="text-lg">{PLATFORM_LABELS[platform] || platform}</strong>
                <Button
                  variant="outline"
                  size="sm"
                  className="ml-auto"
                  onClick={() => handleRefreshPlatform(platform)}
                  disabled={refreshing === platform}
                >
                  {refreshing === platform ? t('channels.refreshing') : t('channels.refresh')}
                </Button>
              </div>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-2">
                {nodes.map((node) => {
                  const peerId = extractPeerId(node.path);
                  return (
                    <div key={node.path} className="rounded-md border px-3 py-2">
                      <div className="text-sm font-medium">
                        {node.displayName || peerId}
                      </div>
                      <div
                        className="text-xs text-muted-foreground mt-0.5 mb-1.5 truncate"
                        title={node.path}
                      >
                        {node.path.length > 40 ? `...${node.path.slice(-37)}` : node.path}
                      </div>
                      {renderAgentSelect(platform, peerId)}
                    </div>
                  );
                })}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      <CreateAgentDialog
        open={showCreateAgent}
        onOpenChange={(open) => {
          setShowCreateAgent(open);
          if (!open) setPendingChannel(null);
        }}
        modelProfiles={modelProfiles}
        onCreated={(result: CreateAgentResult) => {
          refreshAgents();
          if (pendingChannel) {
            handleAssign(pendingChannel.platform, pendingChannel.peerId, result.agentId);
            if (result.persona && pendingChannel.platform === "discord") {
              const ch = (discordChannels || []).find((c) => c.channelId === pendingChannel.peerId);
              if (ch) {
                const path = `channels.discord.guilds.${ch.guildId}.channels.${ch.channelId}.systemPrompt`;
                ua.queueCommand(
                  `Set persona for Discord channel ${ch.channelName || ch.channelId}`,
                  ["openclaw", "config", "set", path, result.persona],
                ).catch((e) => showToast?.(String(e), "error"));
              }
            }
            setPendingChannel(null);
          }
        }}
      />
    </section>
  );
}
