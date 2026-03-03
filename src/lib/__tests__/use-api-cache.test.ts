import { describe, test, expect } from "bun:test";

// Test the cache primitives directly (without React hooks)
// These are the module-level functions that power the caching layer.

// We need to import the internal cache functions.
// Since they're module-scoped, we test via the exported API:
import {
  setOptimisticReadCache,
  readCacheValue,
  buildCacheKey,
  invalidateGlobalReadCache,
} from "../use-api";

describe("buildCacheKey", () => {
  test("builds key with method only", () => {
    const key = buildCacheKey("inst#1", "listAgents");
    expect(key).toBe("inst#1:listAgents:[]");
  });

  test("builds key with args", () => {
    const key = buildCacheKey("inst#1", "getCronRuns", ["job-1", 10]);
    expect(key).toBe('inst#1:getCronRuns:["job-1",10]');
  });

  test("different instances produce different keys", () => {
    const a = buildCacheKey("inst#1", "listAgents");
    const b = buildCacheKey("inst#2", "listAgents");
    expect(a).not.toBe(b);
  });
});

describe("setOptimisticReadCache", () => {
  test("sets and reads a value", () => {
    const key = buildCacheKey("test#optimistic", "myMethod");
    setOptimisticReadCache(key, [{ id: "a" }, { id: "b" }]);
    const result = readCacheValue(key);
    expect(result).toEqual([{ id: "a" }, { id: "b" }]);
  });

  test("overwrites previous cache value", () => {
    const key = buildCacheKey("test#overwrite", "data");
    setOptimisticReadCache(key, "old");
    setOptimisticReadCache(key, "new");
    expect(readCacheValue(key)).toBe("new");
  });

  test("returns undefined for unknown keys", () => {
    expect(readCacheValue("nonexistent:key:[]")).toBeUndefined();
  });
});

describe("invalidateGlobalReadCache", () => {
  test("clears global cache entries", () => {
    const key = buildCacheKey("__global__", "listModelProfiles");
    setOptimisticReadCache(key, [{ id: "p1" }]);
    expect(readCacheValue(key)).toBeDefined();

    invalidateGlobalReadCache(["listModelProfiles"]);
    // After invalidation, the entry should be deleted
    expect(readCacheValue(key)).toBeUndefined();
  });

  test("does not clear non-matching methods", () => {
    const kept = buildCacheKey("__global__", "getAppPreferences");
    const cleared = buildCacheKey("__global__", "listModelProfiles");
    setOptimisticReadCache(kept, "keep-me");
    setOptimisticReadCache(cleared, "clear-me");

    invalidateGlobalReadCache(["listModelProfiles"]);
    expect(readCacheValue(kept)).toBe("keep-me");
    expect(readCacheValue(cleared)).toBeUndefined();
  });
});
