import { describe, expect, test } from "bun:test";

import { formatTime, formatBytes } from "../utils";

describe("formatTime", () => {
  test("parses dash-separated format", () => {
    const result = formatTime("2026-02-17T14-30-00");
    // Should produce a valid date-time string
    expect(result).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    expect(result).toContain("2026");
    expect(result).toContain("14:30:00");
  });

  test("parses ISO 8601 format", () => {
    const result = formatTime("2026-03-01T09:15:30Z");
    expect(result).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    expect(result).toContain("2026");
  });

  test("returns original string for invalid input", () => {
    expect(formatTime("not a date")).toBe("not a date");
  });

  test("returns original for empty string", () => {
    expect(formatTime("")).toBe("");
  });

  test("handles RFC3339 with timezone offset", () => {
    const result = formatTime("2026-01-15T10:00:00+08:00");
    expect(result).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
  });
});

describe("formatBytes", () => {
  test("formats zero bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  test("formats negative bytes as 0 B", () => {
    expect(formatBytes(-100)).toBe("0 B");
  });

  test("formats bytes", () => {
    expect(formatBytes(500)).toBe("500.0 B");
  });

  test("formats kilobytes", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  test("formats megabytes", () => {
    expect(formatBytes(1048576)).toBe("1.0 MB");
  });

  test("formats gigabytes", () => {
    expect(formatBytes(1073741824)).toBe("1.0 GB");
  });

  test("large values stay in GB", () => {
    // 2 TB should still show as GB since GB is the largest unit
    expect(formatBytes(2 * 1073741824)).toBe("2.0 GB");
  });
});
