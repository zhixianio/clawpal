import { SSH_PASSPHRASE_RETRY_HINT } from "./sshConnectErrors";

export function formatSshConnectionLatency(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "-";
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  return `${Math.round(ms)} ms`;
}

export function shouldAutoProbeSshConnectionProfile(params: {
  checked: boolean;
  checking: boolean;
  hasProfile: boolean;
  deferredInteractive: boolean;
}): boolean {
  return !params.checked && !params.checking && !params.hasProfile && !params.deferredInteractive;
}

export function shouldDeferInteractiveSshAutoProbe(rawError: string): boolean {
  return SSH_PASSPHRASE_RETRY_HINT.test(rawError);
}
