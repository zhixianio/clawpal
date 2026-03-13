import { describe, expect, test } from "bun:test";
import { getAction, resolveSteps, stepToCommands } from "../actions";

// Test renderArgs via resolveSteps (renderArgs is not exported directly)
describe("resolveSteps / renderArgs", () => {
  test("substitutes single {{param}} values", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { agentId: "{{name}}" } }];
    const result = resolveSteps(steps, { name: "bot1" });
    expect(result[0].args.agentId).toBe("bot1");
  });

  test("handles boolean coercion for 'true'", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { independent: "{{flag}}" } }];
    const result = resolveSteps(steps, { flag: "true" });
    expect(result[0].args.independent).toBe(true);
  });

  test("handles boolean coercion for 'false'", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { independent: "{{flag}}" } }];
    const result = resolveSteps(steps, { flag: "false" });
    expect(result[0].args.independent).toBe(false);
  });

  test("handles inline {{param}} in longer strings", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { desc: "Agent {{name}} config" } }];
    const result = resolveSteps(steps, { name: "mybot" });
    expect(result[0].args.desc).toBe("Agent mybot config");
  });

  test("passes non-string values through unchanged", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { count: 42, flag: true } }];
    const result = resolveSteps(steps, {});
    expect(result[0].args.count).toBe(42);
    expect(result[0].args.flag).toBe(true);
  });

  test("handles missing params as empty string", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { agentId: "{{missing}}" } }];
    const result = resolveSteps(steps, {});
    expect(result[0].args.agentId).toBe("");
  });

  test("handles multiple params in one string", () => {
    const steps = [{ action: "create_agent", label: "Create", args: { desc: "{{a}}-{{b}}" } }];
    const result = resolveSteps(steps, { a: "hello", b: "world" });
    expect(result[0].args.desc).toBe("hello-world");
  });
});

describe("resolveSteps", () => {
  test("resolves args and computes descriptions", () => {
    const steps = [{
      action: "create_agent",
      label: "Create Agent",
      args: { agentId: "{{name}}", modelProfileId: "__default__" },
    }];
    const result = resolveSteps(steps, { name: "test-bot" });
    expect(result[0].index).toBe(0);
    expect(result[0].action).toBe("create_agent");
    expect(result[0].args.agentId).toBe("test-bot");
    expect(result[0].description).toContain("test-bot");
    expect(result[0].skippable).toBe(false);
  });

  test("marks steps skippable when param is empty string", () => {
    const steps = [{
      action: "create_agent",
      label: "Create Agent",
      args: { agentId: "{{name}}" },
    }];
    const result = resolveSteps(steps, { name: "" });
    expect(result[0].skippable).toBe(true);
  });

  test("non-empty param is not skippable", () => {
    const steps = [{
      action: "create_agent",
      label: "Create Agent",
      args: { agentId: "{{name}}" },
    }];
    const result = resolveSteps(steps, { name: "bot" });
    expect(result[0].skippable).toBe(false);
  });

  test("injects params for config_patch action", () => {
    const steps = [{
      action: "config_patch",
      label: "Patch",
      args: { patchTemplate: '{"key": "{{val}}"}' },
    }];
    const params = { val: "hello" };
    const result = resolveSteps(steps, params);
    expect(result[0].args.params).toEqual(params);
  });

  test("uses label when action has no describe", () => {
    const steps = [{
      action: "nonexistent_action",
      label: "My Custom Label",
      args: {},
    }];
    const result = resolveSteps(steps, {});
    expect(result[0].description).toBe("My Custom Label");
  });

  test("handles multiple steps", () => {
    const steps = [
      { action: "create_agent", label: "Step 1", args: { agentId: "{{a}}" } },
      { action: "setup_identity", label: "Step 2", args: { name: "{{b}}" } },
    ];
    const result = resolveSteps(steps, { a: "bot", b: "Bot Name" });
    expect(result).toHaveLength(2);
    expect(result[0].index).toBe(0);
    expect(result[1].index).toBe(1);
  });
});

describe("getAction", () => {
  test("returns defined actions", () => {
    expect(getAction("create_agent")).toBeDefined();
    expect(getAction("setup_identity")).toBeDefined();
    expect(getAction("set_agent_identity")).toBeDefined();
    expect(getAction("bind_agent")).toBeDefined();
    expect(getAction("ensure_model_profile")).toBeDefined();
    expect(getAction("list_agents")).toBeDefined();
    expect(getAction("bind_channel")).toBeDefined();
    expect(getAction("config_patch")).toBeDefined();
    expect(getAction("set_global_model")).toBeDefined();
  });

  test("returns undefined for unknown actions", () => {
    expect(getAction("nonexistent")).toBeUndefined();
    expect(getAction("")).toBeUndefined();
  });
});

describe("stepToCommands", () => {
  test("throws for unknown action type", async () => {
    const step = {
      index: 0,
      action: "unknown_action",
      label: "Unknown",
      args: {},
      description: "Unknown",
      skippable: false,
    };
    await expect(stepToCommands(step)).rejects.toThrow("Unknown action type: unknown_action");
  });

  test("create_agent does not force a workspace when independent is present", async () => {
    const commands = await stepToCommands({
      index: 0,
      action: "create_agent",
      label: "Create",
      args: { agentId: "mybot", independent: true },
      description: "Create agent",
      skippable: false,
    });

    expect(commands).toEqual([
      ["Create agent: mybot", ["openclaw", "agents", "add", "mybot", "--non-interactive"]],
    ]);
  });
});

describe("Action describe functions", () => {
  test("create_agent describe with default model", () => {
    const action = getAction("create_agent")!;
    const desc = action.describe({ agentId: "mybot", modelProfileId: "__default__" });
    expect(desc).toContain("mybot");
    expect(desc).toContain("default model");
  });

  test("create_agent describe with custom model", () => {
    const action = getAction("create_agent")!;
    const desc = action.describe({ agentId: "mybot", modelProfileId: "gpt-4" });
    expect(desc).toContain("mybot");
    expect(desc).toContain("gpt-4");
  });

  test("create_agent describe ignores legacy independent flag", () => {
    const action = getAction("create_agent")!;
    const desc = action.describe({ agentId: "mybot", independent: true });
    expect(desc).toContain('Create agent "mybot"');
    expect(desc).not.toContain("independent");
  });

  test("setup_identity describe", () => {
    const action = getAction("setup_identity")!;
    const desc = action.describe({ name: "Test Bot", emoji: "🤖" });
    expect(desc).toContain("Test Bot");
  });

  test("setup_identity describe without emoji", () => {
    const action = getAction("setup_identity")!;
    const desc = action.describe({ name: "Test Bot" });
    expect(desc).toContain("Test Bot");
  });

  test("bind_channel describe", () => {
    const action = getAction("bind_channel")!;
    const desc = action.describe({ agentId: "mybot", channelType: "discord" });
    expect(desc).toContain("discord");
    expect(desc).toContain("mybot");
  });

  test("set_agent_identity describe", () => {
    const action = getAction("set_agent_identity")!;
    const desc = action.describe({ agentId: "mybot", name: "My Bot" });
    expect(desc).toContain("mybot");
  });

  test("ensure_model_profile describe", () => {
    const action = getAction("ensure_model_profile")!;
    const desc = action.describe({ profileId: "openai:default" });
    expect(desc).toContain("openai:default");
  });

  test("list_agents describe", () => {
    const action = getAction("list_agents")!;
    const desc = action.describe({});
    expect(desc).toContain("List agents");
  });

  test("set_global_model describe", () => {
    const action = getAction("set_global_model")!;
    const desc = action.describe({ profileId: "gpt-4" });
    expect(desc).toContain("gpt-4");
  });

  test("config_patch describe returns empty string", () => {
    const action = getAction("config_patch")!;
    const desc = action.describe({});
    expect(desc).toBe("");
  });
});
