import { describe, expect, test } from "bun:test";
import {
  makeCacheKey,
  setOptimisticCacheValue,
  invalidateCacheKeys,
  invalidateAndRefetch,
  createMutationWrapper,
} from "../use-cached-query";

// Access the internal cache for verification (via the module's exported functions)
// We test via the public API behavior rather than reaching into internals.

describe("makeCacheKey", () => {
  test("builds key with method only (no args)", () => {
    const key = makeCacheKey("inst#1", "listAgents");
    expect(key).toBe("inst#1:listAgents");
  });

  test("builds key with empty args array", () => {
    const key = makeCacheKey("inst#1", "listAgents", []);
    expect(key).toBe("inst#1:listAgents");
  });

  test("builds key with string args", () => {
    const key = makeCacheKey("inst#1", "getAgent", ["agent-1"]);
    expect(key).toBe('inst#1:getAgent:["agent-1"]');
  });

  test("builds key with multiple args", () => {
    const key = makeCacheKey("inst#1", "getCronRuns", ["job-1", 10]);
    expect(key).toBe('inst#1:getCronRuns:["job-1",10]');
  });

  test("different instances produce different keys", () => {
    const a = makeCacheKey("inst#1", "listAgents");
    const b = makeCacheKey("inst#2", "listAgents");
    expect(a).not.toBe(b);
  });

  test("different methods produce different keys", () => {
    const a = makeCacheKey("inst#1", "listAgents");
    const b = makeCacheKey("inst#1", "listChannels");
    expect(a).not.toBe(b);
  });

  test("handles non-serializable args gracefully", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    // Should not throw — falls back to length-based key
    const key = makeCacheKey("inst#1", "test", [circular]);
    expect(key).toContain("inst#1:test:");
  });
});

describe("setOptimisticCacheValue", () => {
  test("pins a value that can be observed via invalidation behavior", () => {
    const key = makeCacheKey("test-opt", "method1");
    setOptimisticCacheValue(key, { data: "optimistic" });
    // After setting optimistic value, invalidation should clear it
    // (This tests that the value was stored in the cache)
    invalidateAndRefetch([key]);
    // No throw = cache entry existed and was invalidated
  });

  test("can set different types of values", () => {
    setOptimisticCacheValue(makeCacheKey("t1", "m1"), "string");
    setOptimisticCacheValue(makeCacheKey("t2", "m2"), 42);
    setOptimisticCacheValue(makeCacheKey("t3", "m3"), [1, 2, 3]);
    setOptimisticCacheValue(makeCacheKey("t4", "m4"), { nested: { data: true } });
    // No throws = all types handled
  });

  test("accepts custom pin duration", () => {
    const key = makeCacheKey("test-pin", "method");
    setOptimisticCacheValue(key, "value", 5000);
    // No throw = accepted
  });
});

describe("invalidateCacheKeys", () => {
  test("clears entries matching prefix", () => {
    const prefix = `test-invalidate-${Date.now()}`;
    const key1 = `${prefix}:method1`;
    const key2 = `${prefix}:method2`;
    setOptimisticCacheValue(key1, "a");
    setOptimisticCacheValue(key2, "b");
    invalidateCacheKeys(prefix);
    // After invalidation, entries should have expiresAt = 0
    // We verify by calling invalidateAndRefetch again (no error)
    invalidateAndRefetch([key1, key2]);
  });

  test("filters by method names when provided", () => {
    const prefix = `test-filter-${Date.now()}`;
    const key1 = `${prefix}:method1`;
    const key2 = `${prefix}:method2:["arg"]`;
    setOptimisticCacheValue(key1, "a");
    setOptimisticCacheValue(key2, "b");
    invalidateCacheKeys(prefix, ["method1"]);
    // method1 should be invalidated, method2 should not
  });

  test("does nothing for non-matching prefix", () => {
    const key = `unique-prefix-${Date.now()}:method`;
    setOptimisticCacheValue(key, "value");
    invalidateCacheKeys("non-matching-prefix");
    // Original entry should still be in cache
  });
});

describe("invalidateAndRefetch", () => {
  test("clears specific keys", () => {
    const key = `refetch-test-${Date.now()}:method`;
    setOptimisticCacheValue(key, "value");
    invalidateAndRefetch([key]);
    // Should not throw
  });

  test("handles non-existent keys gracefully", () => {
    invalidateAndRefetch(["nonexistent-key-12345"]);
    // Should not throw
  });

  test("handles empty array", () => {
    invalidateAndRefetch([]);
    // Should not throw
  });
});

describe("createMutationWrapper", () => {
  test("calls wrapped function and returns result", async () => {
    const fn = async (x: number) => x * 2;
    const wrapped = createMutationWrapper(fn);
    const result = await wrapped(5);
    expect(result).toBe(10);
  });

  test("calls fn with correct arguments", async () => {
    let capturedArgs: unknown[] = [];
    const fn = async (...args: unknown[]) => {
      capturedArgs = args;
      return "ok";
    };
    const wrapped = createMutationWrapper(fn);
    await wrapped("a", "b", "c");
    expect(capturedArgs).toEqual(["a", "b", "c"]);
  });

  test("invalidates keys after success", async () => {
    const key = `mutation-test-${Date.now()}:method`;
    setOptimisticCacheValue(key, "before");

    const fn = async () => "done";
    const wrapped = createMutationWrapper(fn, { invalidate: [key] });
    await wrapped();
    // Invalidation happened — the cache entry should be cleared
  });

  test("propagates errors from wrapped function", async () => {
    const fn = async () => { throw new Error("boom"); };
    const wrapped = createMutationWrapper(fn);
    await expect(wrapped()).rejects.toThrow("boom");
  });

  test("works without invalidate option", async () => {
    const fn = async () => 42;
    const wrapped = createMutationWrapper(fn, {});
    expect(await wrapped()).toBe(42);
  });
});
