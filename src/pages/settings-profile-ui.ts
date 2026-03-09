import type { ModelProfile } from "@/lib/types";

export type SettingsProfileAction = "edit" | "delete";

export function getSettingsProfileUiState(_profile: ModelProfile): {
  showEnabledBadge: boolean;
  showEnabledToggle: boolean;
  actions: SettingsProfileAction[];
} {
  return {
    showEnabledBadge: false,
    showEnabledToggle: false,
    actions: ["edit", "delete"],
  };
}
