export function isHttpRecipeSource(value: string): boolean {
  return /^https?:\/\//i.test(value.trim());
}

