import { createContext, useContext } from "react";
import type { ChannelNode, DiscordGuildChannel } from "./types";

interface InstanceContextValue {
  instanceId: string;
  instanceLabel?: string | null;
  instanceViewToken: string;
  instanceToken: number;
  persistenceScope: string | null;
  persistenceResolved: boolean;
  isRemote: boolean;
  isDocker: boolean;
  isConnected: boolean;
  channelNodes: ChannelNode[] | null;
  discordGuildChannels: DiscordGuildChannel[] | null;
  channelsLoading: boolean;
  discordChannelsLoading: boolean;
  refreshChannelNodesCache: () => Promise<ChannelNode[]>;
  refreshDiscordChannelsCache: () => Promise<DiscordGuildChannel[]>;
}

export const InstanceContext = createContext<InstanceContextValue>({
  instanceId: "local",
  instanceLabel: "local",
  instanceViewToken: "local",
  instanceToken: 0,
  persistenceScope: "local",
  persistenceResolved: true,
  isRemote: false,
  isDocker: false,
  isConnected: true,
  channelNodes: null,
  discordGuildChannels: null,
  channelsLoading: false,
  discordChannelsLoading: false,
  refreshChannelNodesCache: async () => [],
  refreshDiscordChannelsCache: async () => [],
});

export function useInstance() {
  return useContext(InstanceContext);
}
