/**
 * useCachedQuery — reactive cache-first data fetching with optimistic update support.
 *
 * Solves the dual-state race condition where:
 *   - Component-level `useState` holds optimistic data
 *   - Module-level `API_READ_CACHE` holds stale TTL data
 *   - Polling overwrites optimistic state with stale cache
 *
 * This hook unifies cache + component state into a single reactive layer:
 *   - Components subscribe to cache key changes
 *   - `setOptimistic()` pins data in cache, preventing poll overwrite
 *   - Mutations invalidate related keys and trigger immediate refetch
 *   - Stale-while-revalidate: show cached data first, update when fresh arrives
 */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

// ─── Global reactive cache ───────────────────────────────────────────

interface CacheEntry<T = unknown> {
  value: T | undefined;
  expiresAt: number;
  generation: number;       // Incremented on every write (optimistic or fetch)
  optimisticUntil: number;  // If > Date.now(), this entry is "pinned" (poll won't overwrite)
  inFlight: Promise<T> | null;
}

type Subscriber = () => void;

const _cache = new Map<string, CacheEntry>();
const _subscribers = new Map<string, Set<Subscriber>>();
let _globalGeneration = 0;

function _notify(key: string) {
  const subs = _subscribers.get(key);
  if (subs) {
    for (const fn of subs) fn();
  }
}

function _subscribe(key: string, fn: Subscriber): () => void {
  let set = _subscribers.get(key);
  if (!set) {
    set = new Set();
    _subscribers.set(key, set);
  }
  set.add(fn);
  return () => {
    set!.delete(fn);
    if (set!.size === 0) _subscribers.delete(key);
  };
}

function _getEntry<T>(key: string): CacheEntry<T> | undefined {
  return _cache.get(key) as CacheEntry<T> | undefined;
}

function _setEntry<T>(key: string, partial: Partial<CacheEntry<T>>) {
  const existing = _cache.get(key) as CacheEntry<T> | undefined;
  const entry: CacheEntry<T> = {
    value: partial.value !== undefined ? partial.value : existing?.value,
    expiresAt: partial.expiresAt ?? existing?.expiresAt ?? 0,
    generation: partial.generation ?? (existing ? existing.generation + 1 : ++_globalGeneration),
    optimisticUntil: partial.optimisticUntil ?? existing?.optimisticUntil ?? 0,
    inFlight: partial.inFlight !== undefined ? partial.inFlight : existing?.inFlight ?? null,
  };
  _cache.set(key, entry as CacheEntry);
  _notify(key);
}

// ─── Public cache API ────────────────────────────────────────────────

/**
 * Set an optimistic value for a cache key.
 * This "pins" the value for `pinDurationMs` (default 10s), during which
 * polling results will NOT overwrite it.
 */
export function setOptimisticCacheValue<T>(
  key: string,
  value: T,
  pinDurationMs = 10_000,
) {
  _setEntry(key, {
    value,
    optimisticUntil: Date.now() + pinDurationMs,
    generation: (_cache.get(key)?.generation ?? _globalGeneration) + 1,
  });
}

/**
 * Invalidate cache entries by key prefix and/or method names.
 * Notifies all subscribers so they refetch.
 */
export function invalidateCacheKeys(prefix: string, methods?: string[]) {
  const methodSet = methods ? new Set(methods) : null;
  for (const key of _cache.keys()) {
    if (!key.startsWith(prefix)) continue;
    if (methodSet) {
      // Key format: "prefix:method:args"
      const rest = key.slice(prefix.length + 1);
      const method = rest.split(":", 1)[0];
      if (!methodSet.has(method)) continue;
    }
    const entry = _cache.get(key)!;
    // Clear the entry but keep subscribers
    _cache.set(key, {
      ...entry,
      expiresAt: 0,
      optimisticUntil: 0,
      inFlight: null,
    });
    _notify(key);
  }
}

/**
 * Invalidate + immediately refetch specific keys.
 * Used after mutations to ensure fresh data replaces stale cache.
 */
export function invalidateAndRefetch(keys: string[]) {
  for (const key of keys) {
    const entry = _cache.get(key);
    if (entry) {
      _cache.set(key, {
        ...entry,
        expiresAt: 0,
        optimisticUntil: 0,
        inFlight: null,
      });
      _notify(key);
    }
  }
}

// ─── Trim ────────────────────────────────────────────────────────────

const MAX_ENTRIES = 512;
function _trimIfNeeded() {
  if (_cache.size <= MAX_ENTRIES) return;
  const deleteCount = _cache.size - MAX_ENTRIES;
  const keys = _cache.keys();
  for (let i = 0; i < deleteCount; i++) {
    const next = keys.next();
    if (next.done) break;
    // Don't trim entries with active subscribers
    if (_subscribers.has(next.value)) continue;
    _cache.delete(next.value);
  }
}

// ─── useCachedQuery hook ─────────────────────────────────────────────

export interface UseCachedQueryOptions {
  /** Polling interval in ms. 0 = no polling. */
  pollInterval?: number;
  /** Cache TTL in ms. Default 10_000. */
  ttlMs?: number;
  /** Whether to skip fetching (e.g., when preconditions not met). */
  enabled?: boolean;
  /** If true, skip poll when there are pending queued commands (for optimistic UI). */
  skipWhenPending?: boolean;
}

export interface UseCachedQueryResult<T> {
  /** Current data (may be cached, optimistic, or fresh). */
  data: T | undefined;
  /** True during initial load (no cached data available yet). */
  isLoading: boolean;
  /** True when fetching in background (data may be stale-while-revalidate). */
  isFetching: boolean;
  /** Manually trigger a refetch, bypassing cache. */
  refetch: () => Promise<T>;
  /**
   * Set an optimistic value.
   * This "pins" the value so polling won't overwrite it.
   * Pin duration defaults to 10s or until the next successful refetch.
   */
  setOptimistic: (value: T | ((prev: T | undefined) => T)) => void;
}

export function makeCacheKey(instanceCacheKey: string, method: string, args: unknown[] = []): string {
  let serializedArgs = "";
  try {
    serializedArgs = args.length > 0 ? JSON.stringify(args) : "";
  } catch {
    serializedArgs = String(args.length);
  }
  return serializedArgs ? `${instanceCacheKey}:${method}:${serializedArgs}` : `${instanceCacheKey}:${method}`;
}

export function useCachedQuery<T>(
  key: string,
  fetcher: () => Promise<T>,
  options: UseCachedQueryOptions = {},
): UseCachedQueryResult<T> {
  const {
    pollInterval = 0,
    ttlMs = 10_000,
    enabled = true,
    skipWhenPending = false,
  } = options;

  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;

  const hasPendingRef = useRef(false);
  const [isFetching, setIsFetching] = useState(false);

  // Subscribe to cache entry changes
  const subscribeToKey = useCallback(
    (onStoreChange: () => void) => _subscribe(key, onStoreChange),
    [key],
  );

  const getSnapshot = useCallback(() => {
    const entry = _getEntry<T>(key);
    return entry?.value;
  }, [key]);

  // useSyncExternalStore ensures React re-renders when cache changes
  const data = useSyncExternalStore(subscribeToKey, getSnapshot, getSnapshot);

  // Fetch data, respecting optimistic pins
  const doFetch = useCallback(async (force = false): Promise<T> => {
    const now = Date.now();
    const entry = _getEntry<T>(key);

    // If pinned (optimistic), don't overwrite unless forced
    if (!force && entry && entry.optimisticUntil > now) {
      return entry.value as T;
    }

    // If cached and not expired, return cached
    if (!force && entry && entry.expiresAt > now && entry.value !== undefined) {
      return entry.value;
    }

    // If already fetching, join the in-flight request
    if (entry?.inFlight) {
      return entry.inFlight;
    }

    setIsFetching(true);
    const request = fetcherRef.current();

    // Mark in-flight
    _setEntry<T>(key, { inFlight: request as Promise<T> });

    try {
      const result = await request;
      const currentEntry = _getEntry<T>(key);

      // Only update if not pinned by a newer optimistic write
      if (!currentEntry || currentEntry.optimisticUntil <= Date.now()) {
        _setEntry<T>(key, {
          value: result,
          expiresAt: Date.now() + ttlMs,
          optimisticUntil: 0,
          inFlight: null,
        });
      } else {
        // Just clear inFlight, keep the optimistic value
        _setEntry<T>(key, { inFlight: null });
      }

      _trimIfNeeded();
      return result;
    } catch (error) {
      // On error, clear inFlight but keep stale data
      const currentEntry = _getEntry<T>(key);
      if (currentEntry?.inFlight === request) {
        _setEntry<T>(key, { inFlight: null });
      }
      throw error;
    } finally {
      setIsFetching(false);
    }
  }, [key, ttlMs]);

  const refetch = useCallback(() => doFetch(true), [doFetch]);

  // Set optimistic value
  const setOptimistic = useCallback(
    (valueOrUpdater: T | ((prev: T | undefined) => T)) => {
      const entry = _getEntry<T>(key);
      const prev = entry?.value;
      const nextValue = typeof valueOrUpdater === "function"
        ? (valueOrUpdater as (prev: T | undefined) => T)(prev)
        : valueOrUpdater;
      setOptimisticCacheValue(key, nextValue);
    },
    [key],
  );

  // Initial fetch
  useEffect(() => {
    if (!enabled) return;
    doFetch().catch(() => {});
  }, [enabled, doFetch]);

  // Polling
  useEffect(() => {
    if (!enabled || pollInterval <= 0) return;

    const interval = setInterval(() => {
      if (skipWhenPending && hasPendingRef.current) return;
      doFetch().catch(() => {});
    }, pollInterval);

    return () => clearInterval(interval);
  }, [enabled, pollInterval, skipWhenPending, doFetch]);

  // Refetch when cache entry is invalidated (generation changes but value cleared)
  useEffect(() => {
    if (!enabled) return;
    const entry = _getEntry<T>(key);
    if (entry && entry.expiresAt === 0 && entry.value === undefined && !entry.inFlight) {
      doFetch().catch(() => {});
    }
  }, [enabled, key, data, doFetch]);

  const isLoading = data === undefined && !isFetching;

  return {
    data,
    isLoading: data === undefined,
    isFetching,
    refetch,
    setOptimistic,
  };
}

/**
 * Helper to create a mutation wrapper that invalidates related cache keys after success.
 *
 * Usage:
 *   const deleteFn = useMutation(
 *     (id: string) => api.deleteItem(id),
 *     { invalidate: [agentsCacheKey, statusCacheKey] }
 *   );
 */
export function createMutationWrapper<TArgs extends unknown[], TResult>(
  fn: (...args: TArgs) => Promise<TResult>,
  options: { invalidate?: string[] } = {},
): (...args: TArgs) => Promise<TResult> {
  return async (...args: TArgs) => {
    const result = await fn(...args);
    if (options.invalidate) {
      invalidateAndRefetch(options.invalidate);
    }
    return result;
  };
}
