import { createContext, useContext } from "react";
import type { ChannelNode, DiscordGuildChannel } from "./types";

interface InstanceContextValue {
  instanceId: string;
  instanceToken: number;
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
  instanceToken: 0,
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
