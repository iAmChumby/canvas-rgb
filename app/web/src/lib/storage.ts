// The one place the editor writes browser storage (#579). A preview writes nothing; reads are
// allowed everywhere. `lib/__tests__/storageGuard.test.ts` fails on a write made anywhere else.
//
// A refused write throws as the browser's own call does: every caller keeps its own `catch`.

import { previewActive } from "@/lib/previewMode";

export function storeLocal(key: string, value: string): void {
  if (!previewActive()) window.localStorage.setItem(key, value);
}

export function removeLocal(key: string): void {
  if (!previewActive()) window.localStorage.removeItem(key);
}

export function storeSession(key: string, value: string): void {
  if (!previewActive()) window.sessionStorage.setItem(key, value);
}
