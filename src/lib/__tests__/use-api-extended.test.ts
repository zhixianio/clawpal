import { describe, expect, test } from "bun:test";
import {
  subscribeToCacheKey,
  readCacheValue,
  setOptimisticReadCache,
  buildCacheKey,
  shouldLogRemoteInvokeMetric,
} from "../use-api";

describe("subscribeToCacheKey", () => {
  test("notifies on cache changes", () => {
    let callCount = 0;
    const key = buildCacheKey(`sub-test-${Date.now()}`, "method");
    const unsub = subscribeToCacheKey(key, () => { callCount++; });
    // Trigger a change
    setOptimisticReadCache(key, "new-value");
    expect(callCount).toBeGreaterThan(0);
    unsub();
  });

  test("unsubscribe stops notifications", () => {
    let callCount = 0;
    const key = buildCacheKey(`unsub-test-${Date.now()}`, "method");
    const unsub = subscribeToCacheKey(key, () => { callCount++; });
    unsub();
    // Change after unsubscribe
    setOptimisticReadCache(key, "value-after-unsub");
    expect(callCount).toBe(0);
  });

  test("multiple subscribers all get notified", () => {
    let count1 = 0;
    let count2 = 0;
    const key = buildCacheKey(`multi-sub-${Date.now()}`, "method");
    const unsub1 = subscribeToCacheKey(key, () => { count1++; });
    const unsub2 = subscribeToCacheKey(key, () => { count2++; });
    setOptimisticReadCache(key, "value");
    expect(count1).toBeGreaterThan(0);
    expect(count2).toBeGreaterThan(0);
    unsub1();
    unsub2();
  });

  test("unsubscribing one does not affect others", () => {
    let count1 = 0;
    let count2 = 0;
    const key = buildCacheKey(`partial-unsub-${Date.now()}`, "method");
    const unsub1 = subscribeToCacheKey(key, () => { count1++; });
    const unsub2 = subscribeToCacheKey(key, () => { count2++; });
    unsub1();
    setOptimisticReadCache(key, "value");
    expect(count1).toBe(0);
    expect(count2).toBeGreaterThan(0);
    unsub2();
  });
});

describe("readCacheValue", () => {
  test("returns undefined for missing keys", () => {
    expect(readCacheValue(`nonexistent-${Date.now()}`)).toBeUndefined();
  });

  test("returns value after setOptimisticReadCache", () => {
    const key = buildCacheKey(`read-test-${Date.now()}`, "method");
    setOptimisticReadCache(key, "my-value");
    expect(readCacheValue(key)).toBe("my-value");
  });

  test("returns complex objects", () => {
    const key = buildCacheKey(`read-obj-${Date.now()}`, "method");
    const data = { items: [{ id: 1 }, { id: 2 }], total: 2 };
    setOptimisticReadCache(key, data);
    expect(readCacheValue(key)).toEqual(data);
  });
});

describe("shouldLogRemoteInvokeMetric", () => {
  test("always logs failures", () => {
    expect(shouldLogRemoteInvokeMetric(false, 0)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(false, 100)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(false, 5000)).toBe(true);
  });

  test("always logs slow calls (>= 1500ms)", () => {
    expect(shouldLogRemoteInvokeMetric(true, 1500)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(true, 2000)).toBe(true);
    expect(shouldLogRemoteInvokeMetric(true, 10000)).toBe(true);
  });

  test("samples fast success calls (returns boolean)", () => {
    // Run multiple times to verify it returns a boolean
    const results = Array.from({ length: 100 }, () => shouldLogRemoteInvokeMetric(true, 100));
    expect(results.every((r) => typeof r === "boolean")).toBe(true);
    // With 5% sampling, most should be false (statistically)
    const trueCount = results.filter((r) => r).length;
    expect(trueCount).toBeLessThan(50); // Very unlikely to exceed 50% at 5% sample rate
  });
});
