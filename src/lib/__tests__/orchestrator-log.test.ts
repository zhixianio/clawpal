import { describe, expect, test } from "bun:test";
import {
  readOrchestratorEvents,
  appendOrchestratorEvent,
  clearOrchestratorEvents,
} from "../orchestrator-log";

// Mock localStorage
const storage = new Map<string, string>();
const mockLocalStorage = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => { storage.set(key, value); },
  removeItem: (key: string) => { storage.delete(key); },
  clear: () => { storage.clear(); },
  get length() { return storage.size; },
  key: (_index: number) => null,
};

const STORAGE_KEY = "clawpal_orchestrator_events_v1";
let uuidCounter = 0;

// Set up mocks once at module level
// @ts-expect-error mock
globalThis.window = { localStorage: mockLocalStorage };
// @ts-expect-error mock
globalThis.crypto = { randomUUID: () => `test-uuid-${++uuidCounter}` };

function resetStorage() {
  storage.clear();
  uuidCounter = 0;
}

describe("readOrchestratorEvents", () => {
  test("returns [] when no storage data", () => {
    resetStorage();
    expect(readOrchestratorEvents()).toEqual([]);
  });

  test("parses stored events", () => {
    resetStorage();
    const events = [
      { id: "e1", at: "2024-01-01T00:00:00Z", level: "info", message: "started", instanceId: "inst1" },
    ];
    storage.set(STORAGE_KEY, JSON.stringify(events));
    const result = readOrchestratorEvents();
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("e1");
    expect(result[0].message).toBe("started");
  });

  test("returns [] on parse error", () => {
    resetStorage();
    storage.set(STORAGE_KEY, "not-json");
    expect(readOrchestratorEvents()).toEqual([]);
  });

  test("returns [] for non-array stored value", () => {
    resetStorage();
    storage.set(STORAGE_KEY, JSON.stringify({ not: "array" }));
    expect(readOrchestratorEvents()).toEqual([]);
  });

  test("returns multiple events in order", () => {
    resetStorage();
    const events = [
      { id: "e1", at: "2024-01-01T00:00:00Z", level: "info", message: "first", instanceId: "inst1" },
      { id: "e2", at: "2024-01-01T00:01:00Z", level: "success", message: "second", instanceId: "inst1" },
    ];
    storage.set(STORAGE_KEY, JSON.stringify(events));
    const result = readOrchestratorEvents();
    expect(result).toHaveLength(2);
    expect(result[0].id).toBe("e1");
    expect(result[1].id).toBe("e2");
  });
});

describe("appendOrchestratorEvent", () => {
  test("adds event with auto-generated id and at", () => {
    resetStorage();
    const event = appendOrchestratorEvent({
      level: "info",
      message: "test event",
      instanceId: "inst1",
    });
    expect(event.id).toBe("test-uuid-1");
    expect(event.at).toBeTruthy();
    expect(event.message).toBe("test event");
    expect(event.level).toBe("info");
    expect(event.instanceId).toBe("inst1");
  });

  test("uses provided id and at when given", () => {
    resetStorage();
    const event = appendOrchestratorEvent({
      id: "custom-id",
      at: "2024-06-01T12:00:00Z",
      level: "error",
      message: "custom event",
      instanceId: "inst2",
    });
    expect(event.id).toBe("custom-id");
    expect(event.at).toBe("2024-06-01T12:00:00Z");
  });

  test("persists to localStorage", () => {
    resetStorage();
    appendOrchestratorEvent({
      level: "success",
      message: "persisted",
      instanceId: "inst1",
    });
    const stored = JSON.parse(storage.get(STORAGE_KEY)!);
    expect(stored).toHaveLength(1);
    expect(stored[0].message).toBe("persisted");
  });

  test("appends to existing events", () => {
    resetStorage();
    appendOrchestratorEvent({ level: "info", message: "first", instanceId: "a" });
    appendOrchestratorEvent({ level: "info", message: "second", instanceId: "a" });
    const stored = JSON.parse(storage.get(STORAGE_KEY)!);
    expect(stored).toHaveLength(2);
  });

  test("respects MAX_EVENTS (300) limit", () => {
    resetStorage();
    // Pre-fill with 299 events
    const existing = Array.from({ length: 299 }, (_, i) => ({
      id: `e-${i}`,
      at: new Date().toISOString(),
      level: "info",
      message: `event ${i}`,
      instanceId: "inst1",
    }));
    storage.set(STORAGE_KEY, JSON.stringify(existing));

    // Add two more (should be at 301, trimmed to 300)
    appendOrchestratorEvent({ level: "info", message: "event 299", instanceId: "inst1" });
    appendOrchestratorEvent({ level: "info", message: "event 300", instanceId: "inst1" });

    const stored = JSON.parse(storage.get(STORAGE_KEY)!);
    expect(stored.length).toBeLessThanOrEqual(300);
  });

  test("preserves optional fields", () => {
    resetStorage();
    const event = appendOrchestratorEvent({
      level: "info",
      message: "detailed",
      instanceId: "inst1",
      sessionId: "sess1",
      goal: "diagnose",
      source: "doctor",
      step: "probe",
      state: "running",
      details: "extra details",
    });
    expect(event.sessionId).toBe("sess1");
    expect(event.goal).toBe("diagnose");
    expect(event.source).toBe("doctor");
    expect(event.step).toBe("probe");
    expect(event.state).toBe("running");
    expect(event.details).toBe("extra details");
  });
});

describe("clearOrchestratorEvents", () => {
  test("clears all events when no instanceId", () => {
    resetStorage();
    appendOrchestratorEvent({ level: "info", message: "a", instanceId: "inst1" });
    appendOrchestratorEvent({ level: "info", message: "b", instanceId: "inst2" });
    clearOrchestratorEvents();
    expect(readOrchestratorEvents()).toEqual([]);
  });

  test("filters by instanceId when provided", () => {
    resetStorage();
    appendOrchestratorEvent({ level: "info", message: "keep", instanceId: "inst1" });
    appendOrchestratorEvent({ level: "info", message: "remove", instanceId: "inst2" });
    clearOrchestratorEvents("inst2");
    const remaining = readOrchestratorEvents();
    expect(remaining).toHaveLength(1);
    expect(remaining[0].message).toBe("keep");
    expect(remaining[0].instanceId).toBe("inst1");
  });

  test("does nothing when instanceId not found", () => {
    resetStorage();
    appendOrchestratorEvent({ level: "info", message: "keep", instanceId: "inst1" });
    clearOrchestratorEvents("nonexistent");
    expect(readOrchestratorEvents()).toHaveLength(1);
  });

  test("removes all events for a given instance", () => {
    resetStorage();
    appendOrchestratorEvent({ level: "info", message: "a", instanceId: "inst1" });
    appendOrchestratorEvent({ level: "error", message: "b", instanceId: "inst1" });
    appendOrchestratorEvent({ level: "info", message: "c", instanceId: "inst2" });
    clearOrchestratorEvents("inst1");
    const remaining = readOrchestratorEvents();
    expect(remaining).toHaveLength(1);
    expect(remaining[0].instanceId).toBe("inst2");
  });
});
