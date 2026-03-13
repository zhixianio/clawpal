import { describe, expect, test } from "bun:test";

import type { RecipeParam } from "@/lib/types";
import {
  buildTouchedParamsOnSubmit,
  findFirstInvalidVisibleParamId,
  validateVisibleParamValues,
} from "../param-form-state";

describe("param-form-state", () => {
  const params: RecipeParam[] = [
    {
      id: "agent_id",
      label: "Agent ID",
      type: "string",
      required: true,
    },
    {
      id: "with_persona",
      label: "Use persona",
      type: "boolean",
      required: false,
    },
    {
      id: "persona",
      label: "Persona",
      type: "textarea",
      required: true,
      dependsOn: "with_persona",
    },
  ];

  test("marks visible invalid params touched on submit", () => {
    expect(
      buildTouchedParamsOnSubmit(params, {
        agent_id: "",
        with_persona: "false",
        persona: "",
      }),
    ).toEqual({
      agent_id: true,
    });
  });

  test("skips hidden params when validating submit errors", () => {
    expect(
      validateVisibleParamValues(params, {
        agent_id: "ops-bot",
        with_persona: "false",
        persona: "",
      }),
    ).toEqual({});
  });

  test("returns the first invalid visible param id", () => {
    expect(
      findFirstInvalidVisibleParamId(params, {
        agent_id: "",
        with_persona: "true",
        persona: "",
      }),
    ).toBe("agent_id");
  });
});
