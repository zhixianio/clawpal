declare module "bun:test" {
  export const describe: (name: string, fn: () => void | Promise<void>) => void;
  export const test: (name: string, fn: () => void | Promise<void>) => void;
  export const expect: (...args: any[]) => any;
  export const beforeEach: (fn: () => void | Promise<void>) => void;
  export const afterEach: (fn: () => void | Promise<void>) => void;
  export const beforeAll: (fn: () => void | Promise<void>) => void;
  export const afterAll: (fn: () => void | Promise<void>) => void;

  interface MockFn {
    mock: { calls: any[][]; results: any[] };
    mockRestore(): void;
    mockClear(): void;
    mockResolvedValueOnce(value: any): MockFn;
    mockRejectedValueOnce(value: any): MockFn;
    mockImplementation(fn: (...args: any[]) => any): MockFn;
  }

  export function spyOn(obj: any, method: string): MockFn;
}
