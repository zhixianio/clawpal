export type DoctorConnectionState = "checking" | "connected" | "disconnected";
export type DoctorHandoffEngine = "zeroclaw" | "openclaw";

interface DoctorLaunchGuidanceLike {
  instanceId: string;
  operation: string;
  createdAt: number;
  preferredEngine?: DoctorHandoffEngine;
}

export function resolveEngineConnectionState(params: {
  diagnosing: boolean;
  connected: boolean;
}): DoctorConnectionState {
  if (params.diagnosing) return "checking";
  return params.connected ? "connected" : "disconnected";
}

export function shouldDisableZeroclawStart(params: {
  diagnosing: boolean;
  doctorUiLoaded: boolean;
}): boolean {
  return params.diagnosing || !params.doctorUiLoaded;
}

export function shouldDisableOpenclawStart(params: {
  diagnosing: boolean;
}): boolean {
  return params.diagnosing;
}

export function shouldShowDoctorDisconnectUi(params: {
  engine: "zeroclaw" | "openclaw";
  connected: boolean;
  messageCount: number;
}): boolean {
  if (params.engine === "zeroclaw") return false;
  return !params.connected && params.messageCount > 0;
}

export function resolveDoctorChatConnected(params: {
  engine: "zeroclaw" | "openclaw";
  connected: boolean;
}): boolean {
  if (params.engine === "zeroclaw") return true;
  return params.connected;
}

export function shouldSurfaceDisconnectError(params: {
  engine: "zeroclaw" | "openclaw";
}): boolean {
  return params.engine !== "zeroclaw";
}

export function hasZeroclawSession(params: {
  connected: boolean;
  messageCount: number;
}): boolean {
  return params.connected || params.messageCount > 0;
}

export function buildDoctorLaunchGuidanceKey(
  guidance: Pick<DoctorLaunchGuidanceLike, "instanceId" | "operation" | "createdAt">,
): string {
  return `${guidance.instanceId}:${guidance.operation}:${guidance.createdAt}`;
}

export function resolvePendingDoctorLaunch(params: {
  active: boolean;
  doctorUiLoaded: boolean;
  launchGuidance: DoctorLaunchGuidanceLike | null;
  lastLaunchKey: string | null;
}): {
  shouldQueue: boolean;
  nextLaunchKey: string | null;
  engine: DoctorHandoffEngine | null;
} {
  if (!params.active || !params.doctorUiLoaded || !params.launchGuidance) {
    return {
      shouldQueue: false,
      nextLaunchKey: params.lastLaunchKey,
      engine: null,
    };
  }

  const nextLaunchKey = buildDoctorLaunchGuidanceKey(params.launchGuidance);
  return {
    shouldQueue: nextLaunchKey !== params.lastLaunchKey,
    nextLaunchKey,
    engine: "zeroclaw",
  };
}
