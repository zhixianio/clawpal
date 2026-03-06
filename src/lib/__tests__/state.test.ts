import { describe, expect, test } from "bun:test";
import { reducer, initialState } from "../state";
import type { AppState } from "../state";
import type { DoctorReport } from "../types";

describe("initialState", () => {
  test("has null doctor", () => {
    expect(initialState.doctor).toBeNull();
  });

  test("has empty message", () => {
    expect(initialState.message).toBe("");
  });

  test("has exactly two keys", () => {
    expect(Object.keys(initialState)).toEqual(["doctor", "message"]);
  });
});

describe("reducer", () => {
  const mockReport: DoctorReport = {
    ok: true,
    score: 100,
    issues: [],
  };

  test("handles setDoctor action", () => {
    const next = reducer(initialState, { type: "setDoctor", doctor: mockReport });
    expect(next.doctor).toEqual(mockReport);
    expect(next.message).toBe("");
  });

  test("handles setMessage action", () => {
    const next = reducer(initialState, { type: "setMessage", message: "hello" });
    expect(next.message).toBe("hello");
    expect(next.doctor).toBeNull();
  });

  test("preserves other state when setting doctor", () => {
    const state: AppState = { doctor: null, message: "existing" };
    const next = reducer(state, { type: "setDoctor", doctor: mockReport });
    expect(next.message).toBe("existing");
    expect(next.doctor).toEqual(mockReport);
  });

  test("preserves other state when setting message", () => {
    const state: AppState = { doctor: mockReport, message: "" };
    const next = reducer(state, { type: "setMessage", message: "new" });
    expect(next.doctor).toEqual(mockReport);
    expect(next.message).toBe("new");
  });

  test("returns same state for unknown action", () => {
    const state: AppState = { doctor: mockReport, message: "test" };
    // @ts-expect-error testing unknown action
    const next = reducer(state, { type: "unknown" });
    expect(next).toBe(state);
  });

  test("setDoctor replaces previous doctor", () => {
    const report2: DoctorReport = { ok: false, score: 50, issues: [{ id: "1", code: "C001", severity: "error", message: "fail", autoFixable: true }] };
    const state: AppState = { doctor: mockReport, message: "" };
    const next = reducer(state, { type: "setDoctor", doctor: report2 });
    expect(next.doctor?.ok).toBe(false);
    expect(next.doctor?.score).toBe(50);
  });

  test("setMessage with empty string", () => {
    const state: AppState = { doctor: null, message: "existing" };
    const next = reducer(state, { type: "setMessage", message: "" });
    expect(next.message).toBe("");
  });
});
