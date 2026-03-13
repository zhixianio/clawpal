export function isHttpRecipeSource(value: string): boolean {
  return /^https?:\/\//i.test(value.trim());
}

export function firstDroppedRecipeSource(paths: string[]): string | null {
  for (const value of paths) {
    const trimmed = value.trim();
    if (trimmed.length > 0) {
      return trimmed;
    }
  }
  return null;
}
