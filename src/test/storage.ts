/**
 * Tests: make `localStorage` throw on every call (private mode, blocked site
 * data, a full quota); returns the restore. Replaces the object itself, so it
 * works whichever `Storage` the environment provides.
 */
export function blockStorage(): () => void {
  const fail = () => {
    throw new Error("Storage is blocked.");
  };
  const blocked = {
    get length(): number {
      return fail();
    },
    clear: fail,
    getItem: fail,
    key: fail,
    removeItem: fail,
    setItem: fail,
  } as unknown as Storage;
  const real = globalThis.localStorage;
  const put = (value: Storage) => {
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value,
    });
    if (typeof window !== "undefined" && window !== globalThis) {
      Object.defineProperty(window, "localStorage", {
        configurable: true,
        value,
      });
    }
  };
  put(blocked);
  return () => put(real);
}
