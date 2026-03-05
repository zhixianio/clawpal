export interface WorkspaceTabState {
  openTabIds: string[];
  activeInstance: string;
  inStart: boolean;
  startSection: "overview" | "profiles" | "settings";
}

export function closeWorkspaceTab(state: WorkspaceTabState, id: string): WorkspaceTabState {
  if (!state.openTabIds.includes(id)) return state;

  const openTabIds = state.openTabIds.filter((tabId) => tabId !== id);
  const isClosingActive = state.activeInstance === id;
  const activeInstance = isClosingActive
    ? (openTabIds[openTabIds.length - 1] ?? "local")
    : state.activeInstance;

  if (openTabIds.length === 0) {
    return {
      ...state,
      openTabIds,
      activeInstance,
      inStart: true,
      startSection: "overview",
    };
  }

  return {
    ...state,
    openTabIds,
    activeInstance,
  };
}

export function shouldRenderGuidanceCard<T>(
  guidanceOpen: boolean,
  guidance: T | null,
): guidance is T {
  return guidanceOpen && guidance !== null;
}
