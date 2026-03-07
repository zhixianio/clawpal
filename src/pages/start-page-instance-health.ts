export interface StartPageInstanceHealthSnapshot {
  healthy: boolean | null;
  agentCount: number;
}

export function shouldShowLocalNotInstalled(
  instanceId: string,
  health: StartPageInstanceHealthSnapshot | undefined,
): boolean {
  return instanceId === "local" && health !== undefined && health.healthy !== true;
}
