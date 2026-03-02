export type SshTranslate = (
  key: string,
  options?: Record<string, string | number | boolean>,
) => string;

export const SSH_PASSPHRASE_RETRY_HINT =
  /passphrase|sign_and_send_pubkey|agent refused operation|can't open \/dev\/tty|authentication agent|key is encrypted|encrypted|passphrase required|public key authentication failed|password authentication failed|password is empty|password.*required/i;

export const SSH_PASSPHRASE_REJECT_HINT =
  /bad decrypt|incorrect passphrase|wrong passphrase|passphrase.*failed|decrypt failed/i;

export const SSH_NO_KEY_HINT =
  /no such file|no such key|could not open|not found|cannot find/i;

export const SSH_PUBLIC_KEY_PERMISSION_HINT = /permission denied|public key authentication failed/i;

export function buildSshPassphraseConnectErrorMessage(
  rawError: string,
  hostLabel: string,
  t: SshTranslate,
): string | null {
  if (SSH_PASSPHRASE_REJECT_HINT.test(rawError)) {
    return t("ssh.passphraseValidationFailed", { host: hostLabel });
  }
  if (SSH_NO_KEY_HINT.test(rawError) && /key/i.test(rawError)) {
    return t("ssh.missingKeyFile", { host: hostLabel });
  }
  if (SSH_PUBLIC_KEY_PERMISSION_HINT.test(rawError)) {
    return t("ssh.publicKeyRejected", { host: hostLabel });
  }
  return null;
}

export function buildSshPassphraseCancelMessage(hostLabel: string, t: SshTranslate): string {
  return t("ssh.passphraseCancelled", { host: hostLabel });
}
