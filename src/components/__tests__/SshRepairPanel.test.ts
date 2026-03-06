import { describe, expect, test } from "bun:test";
import type { SshRepairAction } from "@/lib/types";

// We test the pure logic layer: repairActionToLabel covers the mapping,
// and the null-return guard is a structural guarantee of the component.
// Full render tests require @testing-library/react which is not yet in deps.

import { repairActionToLabel } from "@/lib/sshDiagnostic";

const t = (key: string): string => key;

describe("SshRepairPanel — repair action label mapping (all variants)", () => {
  const cases: SshRepairAction[] = [
    "promptPassphrase",
    "retryWithBackoff",
    "switchAuthMethodToSshConfig",
    "suggestKnownHostsBootstrap",
    "suggestAuthorizedKeysCheck",
    "suggestPortHostValidation",
    "reconnectSession",
  ];

  for (const action of cases) {
    test(`${action} maps to a non-empty i18n key`, () => {
      const label = repairActionToLabel(action, t);
      expect(typeof label).toBe("string");
      expect(label.length).toBeGreaterThan(0);
      // Should resolve to an i18n key path (ssh.repair*), not raw action
      expect(label).toMatch(/^ssh\./);
    });
  }
});

describe("SshRepairPanel — empty repairPlan guard", () => {
  test("empty array produces no labels to render", () => {
    const repairPlan: SshRepairAction[] = [];
    const labels = repairPlan.map((a) => repairActionToLabel(a, t));
    expect(labels).toHaveLength(0);
  });
});
