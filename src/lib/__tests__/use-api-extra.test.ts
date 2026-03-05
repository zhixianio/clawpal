import { describe, test, expect } from "bun:test";

import {
  hasGuidanceEmitted,
  subscribeToCacheKey,
  setOptimisticReadCache,
  readCacheValue,
  buildCacheKey,
  invalidateGlobalReadCache,
  shouldLogRemoteInvokeMetric,
} from "../use-api";

describe("hasGuidanceEmitted", () => {
  test("returns true when _guidanceEmitted is true", () => {
    const err = new Error("test");
    (err as any)._guidanceEmitted = true;
    expect(hasGuidanceEmitted(err)).toBe(true);
  });

  test("returns false for plain Error without flag", () => {
    expect(hasGuidanceEmitted(new Error("test"))).toBe(false);
  });

  test("returns false for null", () => {
    expect(hasGuidanceEmitted(null)).toBe(false);
  });

  test("returns false for undefined", () => {
    expect(hasGuidanceEmitted(undefined)).toBe(false);
  });

  test("returns false for non-object (string)", () => {
    expect(hasGuidanceEmitted("some error")).toBe(false);
  });

  test("returns false for non-object (number)", () => {
    expect(hasGuidanceEmitted(42)).toBe(false);
  });

  test("returns false when _guidanceEmitted is falsy", () => {
    const err = new Error("test");
    (err as any)._guidanceEmitted = false;
    expect(hasGuidanceEmitted(err)).toBe(false);
  });

  test("returns true for plain object with _guidanceEmitted", () => {
    expect(hasGuidanceEmitted({ _guidanceEmitted: true })).toBe(true);
  });
});

describe("subscribeToCacheKey", () => {
  test("callback is invoked when cache key is updated", () => {
    const key = buildCacheKey("sub#test1", "method");
    let called = 0;
    const unsub = subscribeToCacheKey(key, () => { called++; });

    // Trigger a cache update which notifies subscribers
    setOptimisticReadCache(key, "value1");
    expect(called).toBe(1);

    setOptimisticReadCache(key, "value2");
    expect(called).toBe(2);

    unsub();
  });

  test("unsubscribe stops further notifications", () => {
    const key = buildCacheKey("sub#test2", "method");
    let called = 0;
    const unsub = subscribeToCacheKey(key, () => { called++; });

    setOptimisticReadCache(key, "v1");
    expect(called).toBe(1);

    unsub();

    setOptimisticReadCache(key, "v2");
    expect(called).toBe(1); // no additional call
  });

  test("multiple subscribers on the same key all get notified", () => {
    const key = buildCacheKey("sub#test3", "method");
    let calledA = 0;
    let calledB = 0;
    const unsubA = subscribeToCacheKey(key, () => { calledA++; });
    const unsubB = subscribeToCacheKey(key, () => { calledB++; });

    setOptimisticReadCache(key, "v1");
    expect(calledA).toBe(1);
    expect(calledB).toBe(1);

    unsubA();
    unsubB();
  });

  test("unsubscribing last listener cleans up the set", () => {
    const key = buildCacheKey("sub#cleanup", "method");
    const unsub = subscribeToCacheKey(key, () => {});
    unsub();

    // After cleanup, a new subscribe should work fine (no stale set)
    let called = 0;
    const unsub2 = subscribeToCacheKey(key, () => { called++; });
    setOptimisticReadCache(key, "fresh");
    expect(called).toBe(1);
    unsub2();
  });
});

describe("buildCacheKey edge cases", () => {
  test("handles circular reference args gracefully", () => {
    const circular: any = { a: 1 };
    circular.self = circular;
    // makeCacheKey catches JSON.stringify errors and falls back to String(args.length)
    const key = buildCacheKey("inst#circ", "method", [circular]);
    expect(key).toBe("inst#circ:method:1");
  });

  test("handles empty args", () => {
    const key = buildCacheKey("inst#1", "op", []);
    expect(key).toBe("inst#1:op:[]");
  });
});

describe("invalidateGlobalReadCache edge cases", () => {
  test("invalidates all global entries when no methods specified", () => {
    const key1 = buildCacheKey("__global__", "methodA");
    const key2 = buildCacheKey("__global__", "methodB");
    setOptimisticReadCache(key1, "a");
    setOptimisticReadCache(key2, "b");

    // No methods filter → invalidate all __global__ entries
    invalidateGlobalReadCache();
    expect(readCacheValue(key1)).toBeUndefined();
    expect(readCacheValue(key2)).toBeUndefined();
  });

  test("notifies subscribers on invalidation", () => {
    const key = buildCacheKey("__global__", "notifyTest");
    setOptimisticReadCache(key, "val");
    let notified = 0;
    const unsub = subscribeToCacheKey(key, () => { notified++; });

    invalidateGlobalReadCache(["notifyTest"]);
    expect(notified).toBe(1);

    unsub();
  });
});

describe("shouldLogRemoteInvokeMetric", () => {
  test("always logs failures regardless of elapsed time", () => {
    expect(shouldLogRemoteInvokeMetric(false, 0)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(false, 100)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(false, 5000)).toBe(true);
  });

  test("always logs slow successful calls (>= 1500ms)", () => {
    expect(shouldLogRemoteInvokeMetric(true, 1500)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(true, 3000)).toBe(true);
  });

  test("fast success calls return a boolean (sampled)", () => {
    // This is probabilistic (5% chance of true), so just verify it returns a boolean
    const result = shouldLogRemoteInvokeMetric(true, 100);
    expect(typeof result).toBe("boolean");
  });
});
