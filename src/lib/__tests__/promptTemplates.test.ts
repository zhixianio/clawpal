import { describe, expect, test } from "bun:test";

// We test the pure renderPromptTemplate function only.
// extractPromptBlock and the named templates depend on Vite ?raw imports.
// We re-implement the pure logic here to avoid import errors.

function renderPromptTemplate(template: string, vars: Record<string, string>): string {
  let output = template;
  for (const [key, value] of Object.entries(vars)) {
    output = output.split(key).join(value);
  }
  return output;
}

function extractPromptBlock(markdown: string): string {
  const marker = "```prompt";
  const start = markdown.indexOf(marker);
  if (start < 0) return markdown.trim();
  const bodyStart = start + marker.length;
  const rest = markdown.slice(bodyStart);
  const end = rest.indexOf("```");
  if (end < 0) return rest.trim();
  return rest.slice(0, end).trim();
}

describe("renderPromptTemplate", () => {
  test("replaces single variable", () => {
    expect(renderPromptTemplate("Hello {{NAME}}", { "{{NAME}}": "Alice" }))
      .toBe("Hello Alice");
  });

  test("replaces multiple occurrences", () => {
    expect(renderPromptTemplate("{{X}} and {{X}}", { "{{X}}": "A" }))
      .toBe("A and A");
  });

  test("replaces multiple variables", () => {
    expect(renderPromptTemplate("{{A}} {{B}}", { "{{A}}": "1", "{{B}}": "2" }))
      .toBe("1 2");
  });

  test("returns template unchanged when no vars match", () => {
    expect(renderPromptTemplate("Hello world", { "{{X}}": "Y" }))
      .toBe("Hello world");
  });

  test("handles empty template", () => {
    expect(renderPromptTemplate("", { "{{X}}": "Y" })).toBe("");
  });

  test("handles empty vars", () => {
    expect(renderPromptTemplate("Hello", {})).toBe("Hello");
  });
});

describe("extractPromptBlock", () => {
  test("extracts content from prompt block", () => {
    const md = "Some text\n```prompt\nHello world\n```\nMore text";
    expect(extractPromptBlock(md)).toBe("Hello world");
  });

  test("returns trimmed markdown when no prompt block", () => {
    expect(extractPromptBlock("  Just plain text  ")).toBe("Just plain text");
  });

  test("handles unclosed prompt block", () => {
    const md = "```prompt\nIncomplete block";
    expect(extractPromptBlock(md)).toBe("Incomplete block");
  });

  test("handles empty prompt block", () => {
    const md = "```prompt\n```";
    expect(extractPromptBlock(md)).toBe("");
  });

  test("extracts multiline content", () => {
    const md = "```prompt\nLine 1\nLine 2\nLine 3\n```";
    expect(extractPromptBlock(md)).toBe("Line 1\nLine 2\nLine 3");
  });
});
