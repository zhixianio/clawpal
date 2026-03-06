export type QuickDiagnoseTransport = "remote_ssh" | "docker_local" | "local";

export function getQuickDiagnoseTransport(isRemote: boolean, isDocker: boolean): QuickDiagnoseTransport {
  if (isRemote) return "remote_ssh";
  if (isDocker) return "docker_local";
  return "local";
}

export function buildPrefillMessage(context: string | null | undefined): string {
  return (context ?? "").trim();
}

export function shouldSeedContext(context: string | null | undefined, alreadySeeded: string): boolean {
  const msg = buildPrefillMessage(context);
  return msg.length > 0 && alreadySeeded !== msg;
}

export function handleQuickDiagnoseDialogOpenChange(
  onOpenChange: (open: boolean) => void,
  open: boolean,
): void {
  onOpenChange(open);
}
