import type { ModelProfile } from "@/lib/types";

interface RecipeModelProfilesApi {
  listRecipeModelProfiles: () => Promise<ModelProfile[]>;
}

export async function loadRecipeModelProfiles(
  api: RecipeModelProfilesApi,
): Promise<ModelProfile[]> {
  const profiles = await api.listRecipeModelProfiles();
  return profiles.filter((profile) => profile.enabled);
}
