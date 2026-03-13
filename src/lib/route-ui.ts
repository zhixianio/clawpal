interface PendingChangesBarVisibility {
  inStart: boolean;
  route: string;
}

export function shouldShowPendingChangesBar({
  inStart,
  route,
}: PendingChangesBarVisibility): boolean {
  if (inStart) return false;
  return route !== "cook";
}
