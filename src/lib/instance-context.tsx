import { createContext, useContext } from "react";
import type { DiscordGuildChannel } from "./types";

interface InstanceContextValue {
  instanceId: string;
  isRemote: boolean;
  isDocker: boolean;
  isConnected: boolean;
  discordGuildChannels: DiscordGuildChannel[];
}

export const InstanceContext = createContext<InstanceContextValue>({
  instanceId: "local",
  isRemote: false,
  isDocker: false,
  isConnected: true,
  discordGuildChannels: [],
});

export function useInstance() {
  return useContext(InstanceContext);
}
