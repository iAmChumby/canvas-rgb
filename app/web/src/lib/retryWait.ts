// The wait before a socket is tried again after it closed: 1.2 s, doubling, up to 15 s.
//
// The helper is its own module because two hooks use it (the per-light feed and the group
// preview, #378) and a test that mocks a hook's module whole would otherwise take the helper
// with it.

/**
 * The wait before a socket is tried again after it closed: 1.2 s, doubling, up to 15 s.
 *
 * The group preview (#378) waits the same way, with this helper and not a copy of it.
 */
export function retryWait(failures: number): number {
  return Math.min(15000, 1200 * Math.pow(2, Math.min(4, failures)));
}
