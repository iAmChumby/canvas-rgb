// #380: the record of the headers the app has already offered by itself.
//
// The record is the header's own and it lives in the viewer's storage. A webview with its
// storage locked throws on every read and on every write: the marks are then kept in memory for
// the life of the page, so a remount in the same window does not offer the header again.

import { afterEach, describe, expect, it } from "vitest";

import { CHAIN_OFFERED_KEY, chainKey, markChainOffered, offeredChains } from "@/views/chain/offered";

const DEVICE = "openrgb:A1B2C3D4E5F6";

const originalStorage = Object.getOwnPropertyDescriptor(window, "localStorage");

/**
 * Puts a storage in the window's place. A storage that throws is a webview whose storage is
 * locked; one that only refuses the writes is a storage that is full.
 */
function useStorage(write: (key: string, value: string) => void, read: (key: string) => string | null): void {
  Object.defineProperty(window, "localStorage", {
    value: {
      getItem: (key: string) => read(key),
      setItem: (key: string, value: string) => write(key, value),
      removeItem: () => undefined,
      clear: () => undefined,
      key: () => null,
      length: 0,
    },
    configurable: true,
    writable: true,
  });
}

/** The window's own storage again. */
function unlockStorage(): void {
  if (originalStorage) Object.defineProperty(window, "localStorage", originalStorage);
  else Reflect.deleteProperty(window, "localStorage");
}

describe("the record of the headers the app has offered", () => {
  afterEach(() => {
    unlockStorage();
    window.localStorage.clear();
  });

  it("is kept in memory when the storage cannot be used", () => {
    const refuse = () => {
      throw new Error("no storage");
    };
    useStorage(refuse, refuse);

    markChainOffered(chainKey(DEVICE, 1));

    // The storage said nothing, but the mark is there: a remount in the same window reads it
    // again and does not offer the header a second time.
    expect(offeredChains().has(chainKey(DEVICE, 1))).toBe(true);
  });

  it("is kept in memory when it cannot be written", () => {
    const written = new Map<string, string>();
    useStorage(
      () => {
        throw new Error("no storage");
      },
      (key) => written.get(key) ?? null,
    );

    markChainOffered(chainKey(DEVICE, 2));

    expect(offeredChains().has(chainKey(DEVICE, 2))).toBe(true);
    expect(written.size).toBe(0);
  });

  it("is written to the storage when it can be used", () => {
    markChainOffered(chainKey(DEVICE, 3));

    const written = JSON.parse(window.localStorage.getItem(CHAIN_OFFERED_KEY) ?? "[]") as string[];
    expect(written).toContain(chainKey(DEVICE, 3));
    expect(offeredChains().has(chainKey(DEVICE, 3))).toBe(true);
  });
});
