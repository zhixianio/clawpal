import { describe, expect, test } from "bun:test";

import { resolveDoctorPageFeatureVisibility } from "../doctor-page-features";

describe("resolveDoctorPageFeatureVisibility", () => {
  test("shows only the formal Doctor Claw surface", () => {
    expect(resolveDoctorPageFeatureVisibility()).toEqual({
      showDoctorClaw: true,
      showOtherAgentHelp: false,
      showRescueBot: false,
    });
  });
});
