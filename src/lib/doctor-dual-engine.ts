export type DoctorConnectionState = "checking" | "connected" | "disconnected";

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

export function hasZeroclawSession(params: {
  connected: boolean;
  messageCount: number;
}): boolean {
  return params.connected || params.messageCount > 0;
}
