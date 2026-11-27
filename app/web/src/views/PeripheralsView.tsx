"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  BatteryCharging,
  BatteryFull,
  ClockCountdown,
  EyeSlash,
  GameController,
  Keyboard,
  Mouse,
  Warning,
} from "@phosphor-icons/react";
import { Button, ButtonLink, Swap } from "@/components/Button";
import { FaultNote } from "@/components/FaultNote";
import { Chip } from "@/components/Chip";
import { Panel, StateDot } from "@/components/Panel";
import { ErrorPanel, LoadFrame, type LoadState } from "@/components/Loading";
import { PeripheralDetailShape, PeripheralListShape } from "@/components/LoadShapes";
import { Toggle } from "@/components/Toggle";
import { Select } from "@/components/Select";
import { InlineSkeleton } from "@/components/Skeleton";
import { ControllerOutline, PAD_STICK_TRAVEL, type PadButton } from "@/components/ControllerOutline";
import {
  devicePadMatch,
  litKeys,
  PAD_STATUS_WORDS,
  PAD_UNREADABLE_WORD,
  STICK_SCALE,
  TRIGGER_SCALE,
  usePadInput,
  type PadFrame,
  type PadMapRow,
} from "@/lib/padInput";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { notify } from "@/components/Notify";
import { UnreadValue } from "@/components/UnreadValue";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { POLL_MS, usePoll } from "@/lib/usePoll";
import {
  choiceWord,
  isUnread,
  known,
  kindWord,
  NOT_READ_WORDS,
  NOT_READ_YET_PROBLEM,
  nounOf,
  READS_OFF_PROBLEM,
  readAgo,
  readStage,
  type PeripheralRow,
} from "@/lib/peripherals";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { ReadProgress } from "@/proto/ReadProgress";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";
import { BarContextMirror, PageActions, PageTitle } from "@/shell/PageHeader";
import { Tooltip } from "@/components/Tooltip";

/** The icon each kind of peripheral wears. */
const KIND_GLYPH = {
  mouse: Mouse,
  game_controller: GameController,
  keyboard: Keyboard,
} as const;

/**
 * True only in a build made for the screenshot harness (#375).
 *
 * Next inlines the value at build time, so the badge below is not in the shipped export at
 * all: the words "Simulated pad" are dropped with the branch that draws them.
 */
const PAD_STUB_BUILT = process.env.NEXT_PUBLIC_PAD_STUB === "1";

/** A label without the profile it belongs to: the panel above it names the profile. */
function plainLabel(label: string): string {
  return label.replace(/ \(profile \d+\)$/, "").replace(/, profile \d+\)$/, ")");
}

/** A schema with only the properties asked for, so a panel can hold part of a section. */
function schemaOf(id: string, name: string, defs: PropertyDef[]): PropertySchema {
  // The panel's title already says which profile: the labels drop the repeat.
  const properties = defs.map((d) => ({ ...d, label: plainLabel(d.label) }));
  return {
    id,
    name,
    description: "",
    needs: { screen: false, audio: false },
    properties,
    presets: [],
  };
}

/** The value a key holds, as a plain thing to print. */
function plain(value: PropValue | undefined): string | number | boolean | null {
  if (!value) return null;
  if (value.type === "number") return value.value;
  if (value.type === "bool") return value.value;
  return value.value;
}

/** True when two values are the same setting. */
function sameValue(a: PropValue | undefined, b: PropValue): boolean {
  if (!a) return false;
  if (a.type !== b.type) return false;
  return a.value === b.value;
}

/** How often the stage of an open read is asked for while the placeholder stands. */
const STAGE_POLL_MS = 300;

/**
 * The connection a device is on. One pad keeps its id when it moves from the 2.4G receiver to
 * Bluetooth, but the two connections hold different settings (#572).
 */
function connectionKey(info: { name: string; location: string }): string {
  return `${info.name}\u0000${info.location}`;
}

/** The cache key of one device on one connection. */
function cacheKey(id: string, info: { name: string; location: string }): string {
  return `${id}\u0000${connectionKey(info)}`;
}

/**
 * The last read of each device on each connection in this session. A device that is picked again
 * shows these values at once, marked as refreshing, while the new read runs. A pad that changed
 * connection must not bring the old connection's sections back.
 */
const lastRead = new Map<string, PeripheralDetail>();

/**
 * Keeps a device's detail for the next time it is picked. A device nothing was read from is not
 * kept: its "Not read yet" frame is not a read, and showing it as the last read would hide the
 * placeholder the page needs while the daemon answers (#127). Changing connection drops the
 * earlier read too, so a no-settings connection never shows the receiver's controls (#572).
 */
function remember(id: string, detail: PeripheralDetail): void {
  const key = cacheKey(id, detail.info);
  const prefix = `${id}\u0000`;
  for (const kept of lastRead.keys()) {
    if (kept.startsWith(prefix) && kept !== key) lastRead.delete(kept);
  }
  if (isUnread(detail)) lastRead.delete(key);
  else lastRead.set(key, detail);
}

/** True on a connection that carries no settings: the daemon describes no section for it. */
function hasNoSettings(detail: PeripheralDetail): boolean {
  return detail.schema.sections.length === 0;
}

/**
 * The head while the detail of a changed connection is on its way. No sections and Change off:
 * the earlier connection's controls must not stay on screen for a pad that just moved (#572).
 * This is a frame, never a cached read: it is not put in `lastRead`.
 */
function pendingDetail(row: PeripheralRow): PeripheralDetail {
  return {
    info: row,
    schema: {
      device: row.id,
      kind: row.kind,
      sections: [],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: {},
    access: { reads: true, writes: false },
    problem: "Reading what this connection holds.",
    read_at_ms: null,
  } as unknown as PeripheralDetail;
}

/**
 * What the page is waiting to see change, or null when it is not waiting: values the daemon has
 * not read yet, or a connection with no settings, which the device can leave at any time.
 */
function waitingFor(detail: PeripheralDetail): string | null {
  if (hasNoSettings(detail)) return `no-settings:${detail.info.name}:${detail.problem ?? ""}`;
  return isUnread(detail) && detail.problem === NOT_READ_YET_PROBLEM ? "unread" : null;
}

/** How often the page asks again while it waits. A plain GET never touches the device. */
export const UNREAD_POLL_MS = 5000;

/** How often the device list is asked again while the page is open, to catch a connection change. */
export const LIST_POLL_MS = POLL_MS;

/** The clock, ticking every `everyMs`, so "Read 3 min ago" moves on by itself. */
function useNowMs(everyMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

/** The entries of `prev` whose device key is in `live`; `prev` itself when nothing goes. */
function keepLive<T>(prev: Record<string, T>, live: Set<string>): Record<string, T> {
  const kept = Object.entries(prev).filter(([id]) => live.has(id));
  return kept.length === Object.keys(prev).length ? prev : Object.fromEntries(kept);
}

/**
 * The stage of the read of `id` that is open now, asked for every `STAGE_POLL_MS` while `active`.
 * The daemon answers at once, also while the read of the device is still waiting for it.
 */
function useReadProgress(id: string | null, active: boolean): ReadProgress | null {
  const [seen, setSeen] = useState<{ id: string; progress: ReadProgress } | null>(null);
  useEffect(() => {
    if (!id || !active) return;
    let stop = false;
    const ask = () => {
      api
        .peripheralRead(id)
        .then((progress) => {
          if (!stop) setSeen({ id, progress });
        })
        .catch(() => {
          // A daemon without the route still reads: the stage line stays "Reading settings".
        });
    };
    const first = setTimeout(ask, 100);
    const timer = setInterval(ask, STAGE_POLL_MS);
    return () => {
      stop = true;
      clearTimeout(first);
      clearInterval(timer);
      setSeen(null);
    };
  }, [id, active]);
  return active && seen && seen.id === id ? seen.progress : null;
}

export function PeripheralsView() {
  const [list, setList] = useState<PeripheralRow[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [pickedId, setSelectedId] = useState<string | null>(null);
  // The detail of the device that is picked. It carries the id it belongs to, so a slow answer
  // never lands on the wrong device.
  const [loaded, setLoaded] = useState<{ id: string; key: string; detail: PeripheralDetail } | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  // The change that is on the device already, and whose refresh failed: the page says the
  // change was saved, because it was.
  const [changeSaved, setChangeSaved] = useState(false);
  // The edits that are not on the device yet, per device key: an edit made on one device is
  // never shown on, or sent to, another (#156).
  const [stagedBy, setStagedBy] = useState<Record<string, Props>>({});
  const setStagedFor = useCallback((id: string, next: Props | ((prev: Props) => Props)) => {
    setStagedBy((prev) => {
      const value = typeof next === "function" ? next(prev[id] ?? {}) : next;
      const out = { ...prev };
      if (Object.keys(value).length > 0) out[id] = value;
      else delete out[id];
      return out;
    });
  }, []);
  const [saving, setSaving] = useState(false);
  // A failed write's message, per device key: one device's read must not clear another
  // device's failure (#156). Only the selected device's own message is shown.
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
  const setDeviceSaveError = useCallback((id: string, message: string | null) => {
    setSaveErrors((prev) => {
      const next = { ...prev };
      if (message) next[id] = message;
      else delete next[id];
      return next;
    });
  }, []);
  const [pickedStage, setPickedStage] = useState<number | null>(null);
  const [pickedButton, setPickedButton] = useState<string | null>(null);

  // The first device is open already: the keys are the way to the others.
  const selectedId = pickedId ?? (list?.[0]?.id ?? null);
  // This device's own failed write's message, if any.
  const saveError = selectedId ? (saveErrors[selectedId] ?? null) : null;
  const staged = useMemo(() => (selectedId ? (stagedBy[selectedId] ?? {}) : {}), [stagedBy, selectedId]);

  // What this device held when it was read: from this page, or from an earlier visit. While the
  // device is read again these stay on screen, marked as refreshing.
  // The list is the only thing that knows the connection before a detail GET lands. Keying the
  // kept read by it means a pad that moved to a connection with no settings shows no stale
  // sections and no enabled Change while the fresh answer is on its way (#572).
  const listed = selectedId ? (list?.find((row) => row.id === selectedId) ?? null) : null;
  const connection = listed ? connectionKey(listed) : null;
  const detailKey = selectedId === null ? null : `${selectedId}\u0000${connection ?? ""}`;
  // A detail this page fetched is authoritative even when the list row is a tick behind it:
  // the no-settings poll can land a connection change before the next list refresh (#572).
  // When the list moves the device to another connection, the answer for the earlier one goes in
  // this render, before anything is painted: dropping it in the fetch effect left one frame of the
  // earlier sections and an enabled Change on screen (#572).
  const [shownKey, setShownKey] = useState(detailKey);
  if (shownKey !== detailKey) {
    setShownKey(detailKey);
    if (shownKey !== null && loaded !== null && loaded.id === selectedId) setLoaded(null);
  }
  const fresh = loaded && loaded.id === selectedId ? loaded.detail : null;
  const kept =
    selectedId !== null && listed ? (lastRead.get(cacheKey(selectedId, listed)) ?? null) : null;
  // The connection changed while this page was open and no answer for it has landed. Show the
  // head with Change off and no sections instead of the other connection's controls. A first
  // visit, where nothing has ever been read here, keeps the skeleton (#572).
  const seenBefore =
    selectedId !== null &&
    Array.from(lastRead.keys()).some((key) => key.startsWith(`${selectedId}\u0000`));
  const pending =
    listed && detailKey !== null && !fresh && !kept && (loaded !== null || seenBefore)
      ? pendingDetail(listed)
      : null;
  const detail = fresh ?? kept ?? pending;
  // The device whose reload is out. It is an id, not a flag: a reload of one device that is
  // still out when another is picked marks only the device it belongs to.
  const [reloadingId, setReloadingId] = useState<string | null>(null);
  const reloading = reloadingId !== null && reloadingId === selectedId;
  const reading = selectedId !== null && !detailError && (fresh === null || reloading);
  const progress = useReadProgress(selectedId, reading);

  // The number of the read that is on its way. An answer that is older than the newest one is
  // dropped: a slow reload of one device must not put its error on another.
  const detailRequest = useRef(0);
  // The connection the last detail fetch was for. A new one drops the detail on screen, so a
  // pad that moved while the page was open cannot keep the earlier connection's sections (#572).
  const fetchedKey = useRef<string | null>(null);

  // A Read again that is out: a second press while it runs is the same read, not another one.
  const rereading = useRef(false);

  // Per device: the number of edits the user has made, and that number when a write to the
  // device last failed. A read that lands keeps the device's unsent edits unless a write failed
  // after the last of them: then the edit the failed write carried goes, so the controls show
  // what the device holds (#151). Unsent edits otherwise survive reads and device switches
  // (#156); a write that lands takes off only the values it sent. A read the page makes never clears a write's message: it knows what the
  // device holds, not how the write ended. The next attempt, an edit, Revert or the user's own
  // Read again clears it.
  const editCount = useRef<Record<string, number>>({});
  const editsAtFailure = useRef<Record<string, number>>({});
  const editsOf = (id: string) => editCount.current[id] ?? 0;
  const keepsStaged = (id: string) => editsOf(id) > (editsAtFailure.current[id] ?? 0);
  // Per device: the number of writes that failed. A Read again clears the message only when it
  // started after the last failure (#151).
  const failures = useRef<Record<string, number>>({});
  const failuresOf = (id: string) => failures.current[id] ?? 0;
  // `carriedEdits`: the failed write sent the staged edits (Apply). An access change that
  // fails carried none of them, so it leaves them staged.
  const noteFailure = (id: string, carriedEdits: boolean) => {
    if (carriedEdits) editsAtFailure.current[id] = editsOf(id);
    failures.current[id] = failuresOf(id) + 1;
  };

  // A read has landed: an edit for a key it did not carry is dropped from the state (#276), so a
  // later read that carries the key again does not bring the old edit back over what the device
  // holds. (`liveStaged` hides such an edit in the meantime.)
  const dropUnreadEdits = useCallback(
    (id: string, d: PeripheralDetail) => {
      setStagedFor(id, (prev) => {
        const out: Props = {};
        for (const [key, value] of Object.entries(prev)) {
          if (known(d.values, key) !== undefined) out[key] = value;
        }
        return out;
      });
    },
    [setStagedFor],
  );

  // Asks for one device's detail again: after a write, or after the access changed (the daemon
  // answers those from the values it keeps), or on the Read again key (`forceRead`: the daemon
  // reads the device). `landed` says that the change is already on the device, so a failed
  // refresh is not a failed change.
  const loadDetail = useCallback(async (id: string, landed = false, forceRead = false) => {
    if (forceRead) {
      if (rereading.current) return;
      rereading.current = true;
    }
    const n = ++detailRequest.current;
    const failed = failuresOf(id);
    setReloadingId(id);
    try {
      const d = forceRead ? await api.peripheral(id, { fresh: true }) : await api.peripheral(id);
      if (n !== detailRequest.current) return;
      remember(id, d);
      setLoaded({ id, key: cacheKey(id, d.info), detail: d });
      dropUnreadEdits(id, d);
      if (!keepsStaged(id)) setStagedFor(id, {});
      // Read again is the user's own new action on this device: it clears the device's failed
      // write's message, when it started after that failure (#151). No other read does.
      if (forceRead && failuresOf(id) === failed) setDeviceSaveError(id, null);
      setDetailError(null);
      setChangeSaved(false);
    } catch (e) {
      if (n !== detailRequest.current) return;
      setLoaded(null);
      setDetailError(messageOf(e));
      setChangeSaved(landed);
    } finally {
      if (forceRead) rereading.current = false;
      if (n === detailRequest.current) setReloadingId(null);
    }
    // editsOf / keepsStaged read refs only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setStagedFor, setDeviceSaveError, dropUnreadEdits]);

  // Lists the devices. The first run and "Try again" both come through here. The stop handle is
  // kept, so a retry that is still running when the page goes away is dropped with it.
  const listStop = useRef<(() => void) | null>(null);
  const [listBusy, setListBusy] = useState(false);

  // `background` is the poll's tick: its failure keeps the list on screen and says nothing, and the
  // next tick asks again (as the rail's count does). Only the first load and "Try again" report.
  const loadList = useCallback((background = false) => {
    let stop = false;
    const handle = () => {
      stop = true;
    };
    listStop.current?.();
    listStop.current = handle;
    api
      .peripherals()
      .then((r) => {
        if (stop) return;
        const rows = r.peripherals as PeripheralRow[];
        setList(rows);
        setListError(null);
        // A device that is gone takes its unsent edits and its message with it, so they do not
        // come back if a device with the same key shows up again.
        const live = new Set(rows.map((row) => row.id));
        setStagedBy((prev) => keepLive(prev, live));
        setSaveErrors((prev) => keepLive(prev, live));
      })
      .catch((e: unknown) => {
        if (!stop && !background) setListError(messageOf(e));
      })
      .finally(() => {
        if (!stop) setListBusy(false);
      });
    return handle;
  }, []);
  useEffect(() => loadList(), [loadList]);
  // A device can move between its receiver and Bluetooth while the page is open. The list is
  // the only place that sees it, so ask again on the live-data beat (#572).
  const pollList = useCallback(() => {
    loadList(true);
  }, [loadList]);
  usePoll(pollList, LIST_POLL_MS);

  // The retry: the frame goes back to its placeholder while the list is asked for again, so the
  // button is gone and a second press cannot start a second load.
  const retryList = () => {
    if (listBusy) return;
    setListError(null);
    setListBusy(true);
    loadList();
  };

  useEffect(() => {
    if (!selectedId) return;
    // The read of a device that is picked again, and again when its connection changes: the
    // id alone is not enough for a pad that moves to Bluetooth while the page is open (#572).
    // The counter drops an answer that belongs to a device that is no longer the one on screen:
    // a slow reload of one device must not put its error on another.
    if (fetchedKey.current !== null && fetchedKey.current !== detailKey) setLoaded(null);
    fetchedKey.current = detailKey;
    const n = ++detailRequest.current;
    const id = selectedId;
    let stop = false;
    api
      .peripheral(id)
      .then((d) => {
        if (stop || n !== detailRequest.current) return;
        // This read is newer than any reload that was out: that reload is dropped when it
        // lands, so its marker is cleared here, or the device would stay "refreshing" for good.
        setReloadingId(null);
        remember(id, d);
        setLoaded({ id, key: cacheKey(id, d.info), detail: d });
        dropUnreadEdits(id, d);
        if (!keepsStaged(id)) setStagedFor(id, {});
        setDetailError(null);
        setChangeSaved(false);
      })
      .catch((e: unknown) => {
        if (stop || n !== detailRequest.current) return;
        setReloadingId(null);
        setLoaded(null);
        setDetailError(messageOf(e));
        setChangeSaved(false);
      });
    return () => {
      stop = true;
    };
    // editsOf / keepsStaged read refs only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, connection, dropUnreadEdits]);

  // "Not read yet": the daemon has read nothing from the device and reads it while the person is
  // away. The page asks again every few seconds with a plain GET (it never touches the device)
  // and stops when values arrive, the device changes or the page goes away (#127).
  // Only while the daemon itself says it has not read the device yet: after a failed Read again
  // the page shows the error and does not poll. A connection with no settings is asked about the
  // same way, so the page follows the device to one that has them.
  const awaiting = detail === null ? null : waitingFor(detail);
  useEffect(() => {
    if (!selectedId || awaiting === null) return;
    const id = selectedId;
    let stop = false;
    const timer = setInterval(() => {
      // A Read again that is out brings its own answer.
      if (rereading.current) return;
      api
        .peripheral(id)
        .then((d) => {
          if (stop || waitingFor(d) === awaiting) return;
          remember(id, d);
          setLoaded({ id, key: cacheKey(id, d.info), detail: d });
          dropUnreadEdits(id, d);
          setDetailError(null);
        })
        .catch(() => {
          // The next tick asks again; a failed poll says nothing new.
        });
    }, UNREAD_POLL_MS);
    return () => {
      stop = true;
      clearInterval(timer);
    };
  }, [selectedId, awaiting, dropUnreadEdits]);

  // An edit for a key the latest read did not carry is dropped, not shown as read (#276): the
  // page offers no control for an unread setting, so the edit can only be left from an earlier
  // read, and writing it would send a value the device was never seen to hold. While no read is
  // on the page (detail is null) the edits are kept as they are.
  const liveStaged = useMemo(() => {
    if (!detail) return staged;
    const out: Props = {};
    for (const [key, value] of Object.entries(staged)) {
      if (known(detail.values, key) !== undefined) out[key] = value;
    }
    return out;
  }, [staged, detail]);
  // The values the controls show: what the device holds, with the staged edits on top.
  const values: Values = useMemo(
    () => ({ ...(detail?.values ?? {}), ...liveStaged }) as Values,
    [detail, liveStaged],
  );

  const changed = useMemo(() => {
    const out: Props = {};
    for (const [key, value] of Object.entries(liveStaged)) {
      if (!sameValue(detail?.values[key], value)) out[key] = value;
    }
    return out;
  }, [liveStaged, detail]);

  // The edits that are not on the device yet: they are the ones the strip counts.
  const changedCount = Object.keys(changed).length;

  const onChange = (key: string, value: PropValue) => {
    if (!selectedId) return;
    editCount.current[selectedId] = editsOf(selectedId) + 1;
    setStagedFor(selectedId, (prev) => ({ ...prev, [key]: value }));
    // An edit replaces what the failed write asked for: its message goes.
    setDeviceSaveError(selectedId, null);
  };

  const apply = async () => {
    if (!detail) return;
    setSaving(true);
    const id = detail.info.id;
    const sent = changed;
    setDeviceSaveError(id, null);
    try {
      await api.patchPeripheral(id, sent);
    } catch (e) {
      // The change itself failed: nothing was changed.
      noteFailure(detail.info.id, true);
      setDeviceSaveError(detail.info.id, messageOf(e));
      setSaving(false);
      return;
    }
    // The change is on the device: the values it sent are no longer staged. An edit made while
    // it was out (a new value, or another setting) stays staged.
    setStagedFor(id, (prev) => {
      const out: Props = {};
      for (const [key, value] of Object.entries(prev)) {
        if (!(key in sent) || !sameValue(sent[key], value)) out[key] = value;
      }
      return out;
    });
    // The read that follows only refreshes the page: when it fails, the change is still saved.
    await loadDetail(id, true);
    notify("The settings are on the device");
    setSaving(false);
  };

  const revert = () => {
    if (!selectedId) return;
    setStagedFor(selectedId, {});
    setDeviceSaveError(selectedId, null);
  };

  const setAccess = async (next: { reads?: boolean; writes?: boolean }) => {
    if (!detail) return;
    const reads = next.reads ?? detail.access.reads;
    const writes = next.writes ?? detail.access.writes;
    // A new attempt: the last failure's message goes.
    setDeviceSaveError(detail.info.id, null);
    try {
      await api.putPeripheralAccess(detail.info.id, { reads, writes });
    } catch (e) {
      // The change itself failed: nothing was changed.
      noteFailure(detail.info.id, false);
      setDeviceSaveError(detail.info.id, messageOf(e));
      return;
    }
    // The access is saved. The read that follows only refreshes the page.
    await loadDetail(detail.info.id, true);
  };

  const rows = list ?? [];
  const noun = nounOf(rows.find((r) => r.id === selectedId)?.kind ?? "mouse");
  // The detail pane is one frame in every state: the placeholder, the error and the empty state
  // are as tall as the panels they stand for, and a device read before keeps its values on
  // screen, marked as refreshing, while it is read again.
  const detailState: LoadState = detailError ? "error" : detail ? "ready" : "loading";
  // With no device to show (the list is empty or could not be read) the list says so, and there is
  // no page to hold a place for.
  const noDetail = listError !== null || (list !== null && rows.length === 0);
  const pickedKind = rows.find((r) => r.id === selectedId)?.kind ?? rows[0]?.kind ?? "mouse";
  const detailPane = noDetail ? null : (
    <LoadFrame
      state={detailState}
      shape={<PeripheralDetailShape kind={pickedKind} />}
      stage={reading ? readStage(progress, selectedId ? noun : "device") : null}
      refreshing={detail !== null && reading}
      label="Reading the device"
      error={
        <ErrorPanel
          title="The device could not be read"
          summary={
            changeSaved
              ? // The change is on the device; only the page could not be refreshed.
                "The change was saved, but the page could not be refreshed."
              : "canvas-rgb could not read that device. Nothing was changed."
          }
          message={detailError}
          onRetry={() => {
            if (!selectedId) return;
            setDetailError(null);
            setChangeSaved(false);
            void loadDetail(selectedId);
          }}
        />
      }
    >
      {detail && (
        <PeripheralDetail
          detail={detail}
          values={values}
          pickedStage={pickedStage}
          pickedButton={pickedButton}
          onPickStage={setPickedStage}
          onPickButton={setPickedButton}
          onChange={onChange}
          onAccess={(next) => void setAccess(next)}
          reading={reading}
          onReadAgain={() => selectedId && void loadDetail(selectedId, false, true)}
        />
      )}
    </LoadFrame>
  );

  const listState: LoadState =
    listError && !listBusy ? "error" : list === null ? "loading" : rows.length === 0 ? "empty" : "ready";

  return (
    <div className="rd-page rd-peripherals">
      <PageTitle title="Peripherals" contextBusy={list === null && !listError} context={list === null && !listError ? <InlineSkeleton width="8ch" /> : <>{rows.length} found</>} />
      {list && <BarContextMirror className="fig">{rows.length} found</BarContextMirror>}
      {/* The pending changes and the way to send them sit in the strip, beside the save (R4). */}
      <PageActions>
        {changedCount > 0 && (
          <>
            <span
              className="warn-text fig rd-peri-pending"
              aria-label={`${changedCount} change${changedCount === 1 ? "" : "s"} not sent`}
              title={`${changedCount} change${changedCount === 1 ? "" : "s"} not sent`}
            >
              <Warning size={14} weight="fill" aria-hidden="true" />
              <span className="rd-peri-pending-text">
                {changedCount} change{changedCount === 1 ? "" : "s"} not sent
              </span>
            </span>
            <Button
              className="rd-secondary"
              variant="ghost"
              size="sm"
              icon={<ArrowCounterClockwise size={16} />}
              disabled={saving}
              aria-label="Revert"
              onClick={revert}
            >
              <span className="rd-peri-revert-label">Revert</span>
            </Button>
            <Button data-bar-primary="" data-short="Apply" variant="primary" size="sm" busy={saving} onClick={() => void apply()}>
              Apply
            </Button>
          </>
        )}
      </PageActions>

      {/* The device column becomes a row of keys: one click, as now. */}
      <div className="rd-peri-keys">
        <LoadFrame
          className="load-col"
          state={listState}
          shape={<PeripheralListShape />}
          label="Looking for devices"
          error={
            <ErrorPanel
              title="The peripherals could not be listed"
              summary="canvas-rgb could not look for the peripherals. Nothing was changed."
              message={listError}
              onRetry={retryList}
            />
          }
          empty={
            <Panel title="No device found">
              <p className="mute">None that canvas-rgb can configure is plugged in.</p>
            </Panel>
          }
        >
          {rows.map((p) => {
            const Glyph = KIND_GLYPH[p.kind];
            const battery = p.battery ?? null;
            return (
              <button
                key={p.id}
                type="button"
                // `tab-key` is the shared type of the app's tabs (#373): the device key is set
                // like every other tab, and this page sets only its box.
                className="peripheral-row rd-devkey tab-key"
                data-on={p.id === selectedId ? "true" : "false"}
                aria-current={p.id === selectedId ? "true" : undefined}
                onClick={() => {
                  setDetailError(null);
                  setChangeSaved(false);
                  // A new device: the stage and the button that were picked belong to the
                  // one that was on screen.
                  setPickedStage(null);
                  setPickedButton(null);
                  setSelectedId(p.id);
                }}
              >
                <span className="peripheral-icon" aria-hidden="true">
                  <Glyph size={18} />
                </span>
                <span className="peripheral-text">
                  <span className="peripheral-name">{p.name}</span>
                </span>
                {battery !== null && (
                  <span className="peripheral-battery fig">
                    {p.charging ? <BatteryCharging size={14} /> : <BatteryFull size={14} />}
                    {battery}%
                  </span>
                )}
                <span
                  className="state-dot"
                  style={{ "--tone": p.connected ? "var(--ok)" : "var(--mute)" } as CSSProperties}
                  aria-hidden="true"
                />
              </button>
            );
          })}
        </LoadFrame>
      </div>

      {saveError && (
        <p className="rule rule-warn" role="status">
          <span>{saveError}</span>
        </p>
      )}

      <div className="peripheral-detail">
        {detailPane}
      </div>
    </div>
  );
}

/** The detail of one device: what it is, what may be done with it, and its settings. */
function PeripheralDetail({
  detail,
  values,
  pickedStage,
  pickedButton,
  onPickStage,
  onPickButton,
  onChange,
  onAccess,
  reading,
  onReadAgain,
}: {
  reading: boolean;
  onReadAgain: () => void;
  detail: PeripheralDetail;
  values: Values;
  pickedStage: number | null;
  pickedButton: string | null;
  onPickStage: (n: number | null) => void;
  onPickButton: (key: string | null) => void;
  onChange: (key: string, value: PropValue) => void;
  onAccess: (next: { reads?: boolean; writes?: boolean }) => void;
}) {
  const info = detail.info;
  const Glyph = KIND_GLYPH[info.kind];
  const readOnly = detail.schema.read_only;
  const writes = detail.access.writes;
  const reads = detail.access.reads;
  const isController = detail.schema.kind === "game_controller";
  const isMouse = detail.schema.kind === "mouse";
  // No settings on this connection: one frame says why, in the daemon's words, and nothing that
  // reads or changes a setting can be pressed. The saved access is left as it is.
  const noSettings = hasNoSettings(detail);
  // Nothing was read yet: a frame that says so replaces the controls (no setting is made up).
  const unread = !noSettings && isUnread(detail);
  // Reads off: the daemon has read nothing and will read nothing until Read is turned on, so the
  // controls have no values to show and stand behind a frame that says what reading does (#264).
  const readsOff = !noSettings && !reads;
  const showsSettings = !noSettings && !unread && !readsOff;
  const noun = nounOf(info.kind);
  // The "Read again" tooltip (#374). Only the mouse's read is known to hitch: it is five report
  // exchanges with the pointer's own reports among them. The pad's read is 36 chunks of its
  // profile image with no pointer in it, so the pad's tooltip promises the read and no more
  // (#387).
  const readAgainWords = noSettings
    ? `The ${noun}'s settings cannot be read over this connection.`
    : isMouse
    ? `Reads the settings from the ${noun} again, to pick up changes made on the ${noun} itself. The ${noun} may stutter briefly while it reads.`
    : `Reads the settings from the ${noun} again, to pick up changes made on the ${noun} itself.`;
  const readWords = noSettings
    ? `Opens the ${noun}. Its settings cannot be read over this connection.`
    : `Opens the ${noun} and asks for its settings. It changes nothing on the ${noun}.`;
  const nowMs = useNowMs(30_000);
  const readAt = detail.read_at_ms == null ? null : Number(detail.read_at_ms);
  const ago = readAt === null ? null : readAgo(readAt, nowMs);
  // The Read switch, in the head and in the reads-off frame: one action, so both stay in step.
  const onReadToggle = (v: boolean) => onAccess({ reads: v, writes: v ? undefined : false });

  // The panels that stand in columns, and the one that spans them all.
  const columnPanels: ReactNode[] = [];
  const widePanels: ReactNode[] = [];

  for (const section of showsSettings ? detail.schema.sections : []) {
    if (isMouse && section.id === "dpi") {
      columnPanels.push(
        <DpiPanel
          key={section.id}
          section={section}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          picked={pickedStage}
          onPick={onPickStage}
          onChange={onChange}
        />,
      );
      continue;
    }
    if (isController && section.id === "buttons") {
      // The outline of the pad and the mapping of its buttons belong together, and they are
      // wider than a column: they span them all.
      widePanels.push(
        <ControllerPanel
          key={section.id}
          detail={detail}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          picked={pickedButton}
          onPick={onPickButton}
          onChange={onChange}
        />,
      );
      continue;
    }
    // One profile shows at a time: the active one.
    if (isController && section.id.startsWith("profile-")) {
      // Which profile is active is read from the device. When the read did not carry it, no
      // profile is drawn as the active one (#276); the profiles key above shows it as not read.
      const active = plain(known(values, "active-profile"));
      if (active === null || section.id !== `profile-${String(active)}`) continue;
    }
    // The paddles are in the mapping list of the plan; the profile panels hold the rest.
    const properties =
      isController && section.id.startsWith("profile-")
        ? section.properties.filter((p) => !p.key.includes("-paddle-"))
        : section.properties;
    columnPanels.push(
      <Panel
        key={section.id}
        title={section.title}
        className={section.id === "buttons" ? "rd-peri-buttons" : undefined}
      >
        <PropertyControls
          fromDevice
          schema={schemaOf(`peripheral-${section.id}`, section.title, properties)}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          optionLabel={choiceWord}
          onChange={onChange}
        />
      </Panel>,
    );
  }

  return (
    <div className="stack">
      {/* The device head, with Access merged into it: what may be done with it is a switch
          beside the device it belongs to (R4). */}
      <Panel>
        <div className="peripheral-head">
          <span className="peripheral-head-icon" aria-hidden="true">
            <Glyph size={22} />
          </span>
          <div className="peripheral-head-text">
            <h2 className="peripheral-head-name">{info.name}</h2>
            <p className="peripheral-head-sub" title={`${kindWord(info.kind)} · ${info.vendor} · ${info.location}`}>
              {kindWord(info.kind)} · {info.vendor} · <span className="fig">{info.location}</span>
            </p>
          </div>
          <StateDot tone={info.connected ? "ok" : "mute"}>
            {info.connected ? "Connected" : "Not connected"}
          </StateDot>
          <div className="rd-peri-access">
            {/* One cell for every wording, so the head keeps its width between states; it is
                there, empty, while reads are off or an older daemon sends no read time. */}
            <span
              className="rd-peri-readat"
              data-role="read-at"
              data-stale={!noSettings && reads && ago?.stale ? "true" : "false"}
              data-empty={!noSettings && reads && (ago || unread) ? "false" : "true"}
            >
              {/* The third wording is never shown: it holds the width of the longest one. */}
              <Swap
                labels={["Not read yet", "Read 3 min ago", "Read 59 min ago"]}
                at={reads && ago ? 1 : 0}
                shown={reads && ago ? ago.text : undefined}
              />
            </span>
            <Tooltip label={readWords} className="rd-peri-tip">
              <span className="rd-peri-toggle">
                <span className="rd-peri-toggle-label">Read</span>
                <Toggle
                  label="Read settings from this device"
                  checked={reads}
                  onChange={onReadToggle}
                />
              </span>
            </Tooltip>
            {/* What a read does, in the words of the code (#374): the editor asks the daemon for
                the device with `?fresh=true` (`GET /peripherals/{id}`, the daemon's
                `crates/canvas-daemon/src/api/peripherals.rs`), the daemon opens the device and
                reads its reports (`read_fresh_stamped`, `crates/canvas-daemon/src/peripherals.rs`),
                and the adapter's read is a series of short report exchanges with a pause between
                them (`crates/canvas-adapters/src/attack_shark_x6/mod.rs`: five reports, a 250 ms
                wait for each grant and a 500 ms pause between the reports). The device is busy
                with those exchanges while the read runs and is usable again when it ends; it is
                never held still for the whole read. The wording says that, and says reading.
                The stutter is the mouse's own report exchange and is claimed for the mouse only
                (#387): the pad's read is 36 chunks of its profile image
                (`crates/canvas-adapters/src/eightbitdo_ultimate2/mod.rs`) with no pointer in it,
                and nothing has measured what its buttons do during one, so its tooltip promises
                only the read. */}
            <Tooltip label={readAgainWords} className="rd-peri-tip">
              <span className="rd-peri-toggle">
                <Button
                  size="sm"
                  variant={unread ? "primary" : "default"}
                  icon={<ArrowsClockwise size={16} />}
                  disabled={noSettings || !reads || reading}
                  onClick={onReadAgain}
                >
                  Read again
                </Button>
              </span>
            </Tooltip>
            <Tooltip
              label={
                noSettings
                  ? `The ${noun}'s settings cannot be changed over this connection.`
                  : `Lets canvas-rgb change the settings on the ${noun}.`
              }
              className="rd-peri-tip"
            >
              <span className="rd-peri-toggle">
                <span className="rd-peri-toggle-label">Change</span>
                <Toggle
                  label="Change settings on the device"
                  checked={!noSettings && writes}
                  disabled={noSettings || !reads}
                  onChange={(v) => onAccess({ writes: v })}
                />
              </span>
            </Tooltip>
          </div>
        </div>
        {detail.problem && !noSettings && !(unread && detail.problem === NOT_READ_YET_PROBLEM) && !(readsOff && detail.problem === READS_OFF_PROBLEM) && (
          <p className="rule rule-warn" data-role="problem">
            {detail.problem}
          </p>
        )}
      </Panel>

      {noSettings && (
        <div data-role="no-settings">
          <FaultNote
            parts={{
              affected: `The ${noun}'s settings cannot be read or changed over this connection.`,
              cause: detail.problem,
              // C3: pad-specific advice only for a game controller. The spine has no typed
              // personality field for Bluetooth, so that one branch still follows the adapter's
              // plain sentence; every other kind gets a neutral next step.
              fix: info.kind === "game_controller"
                ? (detail.problem ?? "").startsWith("Over Bluetooth")
                  ? "Connect through the 2.4G receiver or a USB cable to change its profiles."
                  : "Switch to XInput mode and connect through the 2.4G receiver or a USB cable."
                : `Check the ${noun}'s own software, then press Read again.`,
              ignore_if: info.kind === "game_controller"
                ? "You only need this pad's buttons, sticks and triggers."
                : `You do not need to change this ${noun}'s settings.`,
            }}
          >
            <ButtonLink href="/help/" size="sm" variant="ghost">
              Why isn&apos;t my device here?
            </ButtonLink>
          </FaultNote>
        </div>
      )}

      {unread && (
        <Panel className="rd-peri-unread">
          <div className="rd-peri-unread-body" data-role="not-read">
            <span className="rd-peri-unread-icon" aria-hidden="true">
              <ClockCountdown size={22} />
            </span>
            <p className="rd-peri-unread-text">
              {/* The mouse is never read in the background (#567), so its frame promises no read. */}
              {isMouse
                ? `Not read yet. Press Read again to read the ${noun}'s settings.`
                : `Not read yet. The ${noun}'s settings are read while you are away. Press Read again to read them now.`}
            </p>
          </div>
        </Panel>
      )}

      {/* Reads off: the same frame as "Not read yet", with one key that starts reading (the head's
          Read switch stays the one place to turn it off: a switch that could only turn on would be a
          button in disguise). No section is drawn, so the page never shows a default as if read. */}
      {readsOff && (
        <Panel className="rd-peri-unread rd-peri-reads-off">
          <div className="rd-peri-unread-body" data-role="reads-off">
            <span className="rd-peri-unread-icon" aria-hidden="true">
              <EyeSlash size={22} />
            </span>
            <p className="rd-peri-unread-text">
              {`The ${noun}'s settings are read only when you turn reading on.`}
            </p>
            <span className="rd-peri-reads-off-action">
              <Tooltip label={`Opens the ${noun} and asks for its settings. It changes nothing on the ${noun}.`}>
                <Button variant="primary" size="sm" onClick={() => onReadToggle(true)}>
                  Turn on reading
                </Button>
              </Tooltip>
            </span>
          </div>
        </Panel>
      )}

      {/* The sections in columns: a panel never breaks across two of them. */}
      {showsSettings && <div className="rd-peri-sections">{columnPanels}</div>}

      {widePanels}

      {isController && showsSettings && (
        <Panel title="RGB rings">
          <p className="mute">
            Rings hold one still colour set per scene. A scene change writes it to the pad while you are away, when this
            pad&rsquo;s reads and writes are on. The app cannot see the rings&rsquo; light.
          </p>
        </Panel>
      )}
    </div>
  );
}

/** The mouse's DPI section: the stages as chips, and the picked stage's two settings. */
function DpiPanel({
  section,
  values,
  readOnly,
  disabled,
  picked,
  onPick,
  onChange,
}: {
  section: PeripheralDetail["schema"]["sections"][number];
  values: Values;
  readOnly: string[];
  disabled: boolean;
  picked: number | null;
  onPick: (n: number | null) => void;
  onChange: (key: string, value: PropValue) => void;
}) {
  // Every figure here is what the device reported or nothing (#276): a stage count, a DPI, a
  // colour or an active stage the read did not carry is null and draws as not read.
  const countRead = plain(known(values, "dpi-stage-count"));
  const count = typeof countRead === "number" ? countRead : null;
  const activeRead = plain(known(values, "dpi-active-stage"));
  const active = activeRead === null ? null : String(activeRead);
  const stages: { n: number; dpi: number | null; colour: string | null }[] = [];
  for (let i = 1; i <= (count ?? 0); i++) {
    const dpiRead = plain(known(values, `dpi-stage-${i}`));
    const colourRead = plain(known(values, `dpi-stage-${i}-colour`));
    stages.push({
      n: i,
      dpi: typeof dpiRead === "number" ? dpiRead : null,
      colour: typeof colourRead === "string" ? colourRead : null,
    });
  }
  const others = section.properties.filter((p) => !p.key.startsWith("dpi-stage-"));
  const pickedDefs = picked
    ? section.properties.filter(
        (p) => p.key === `dpi-stage-${picked}` || p.key === `dpi-stage-${picked}-colour`,
      )
    : [];

  return (
    <Panel title={section.title}>
      <PropertyControls
        fromDevice
        schema={schemaOf("dpi-count", "DPI", others)}
        values={values}
        readOnly={readOnly}
        disabled={disabled}
        optionLabel={choiceWord}
        onChange={onChange}
      />
      {/* The stage count is on the element so the columns are chosen from it (#387): the row is a
          grid of equal cells, and a count that cannot fill its last row evenly (four, which would
          be 3 + 1) takes two columns instead. */}
      <div className="dpi-stages" role="group" aria-label="DPI stages" data-stages={stages.length}>
        {count === null && <UnreadValue label="DPI stages" />}
        {stages.map((s) => (
          <button
            key={s.n}
            type="button"
            className="dpi-chip"
            data-on={picked === s.n ? "true" : "false"}
            data-active={active === String(s.n) ? "true" : "false"}
            aria-pressed={picked === s.n}
            aria-label={`DPI stage ${s.n}, ${s.dpi === null ? NOT_READ_WORDS : s.dpi}${active === String(s.n) ? ", the active stage" : ""}`}
            title={s.dpi === null ? NOT_READ_WORDS : undefined}
            onClick={() => onPick(picked === s.n ? null : s.n)}
          >
            <span
              className="dpi-chip-dot"
              data-unread={s.colour === null ? "true" : "false"}
              style={s.colour === null ? undefined : ({ "--c": s.colour } as CSSProperties)}
              aria-hidden="true"
            />
            <span className="fig">{s.dpi ?? "—"}</span>
          </button>
        ))}
      </div>
      {picked !== null && pickedDefs.length > 0 && (
        <div className="dpi-stage-edit">
          <PropertyControls
            fromDevice
            schema={schemaOf(`dpi-stage-${picked}`, `DPI stage ${picked}`, pickedDefs)}
            values={values}
            readOnly={readOnly}
            disabled={disabled}
            onChange={onChange}
          />
        </div>
      )}
    </Panel>
  );
}

/** The controller: the plan of the pad, and the mapping of every button. */
function ControllerPanel({
  detail,
  values,
  readOnly,
  disabled,
  picked,
  onPick,
  onChange,
}: {
  detail: PeripheralDetail;
  values: Values;
  readOnly: string[];
  disabled: boolean;
  picked: string | null;
  onPick: (key: string | null) => void;
  onChange: (key: string, value: PropValue) => void;
}) {
  const buttons = detail.schema.sections.find((s) => s.id === "buttons");
  // The paddles belong to the active profile; with no active profile read, none are listed (#276).
  const profile = plain(known(values, "active-profile"));
  const profileSection =
    profile === null ? undefined : detail.schema.sections.find((s) => s.id === `profile-${String(profile)}`);
  const paddles = useMemo(
    () => (profileSection?.properties ?? []).filter((p) => p.key.includes("-paddle-")),
    [profileSection],
  );
  const rows: PropertyDef[] = useMemo(
    () => [...(buttons?.properties ?? []), ...paddles],
    [buttons, paddles],
  );
  const rowRefs = useRef(new Map<string, HTMLDivElement | null>());

  const planButtons: PadButton[] = rows.map((p) => ({
    key: p.key,
    label: p.label,
    print: p.label.length <= 5 ? p.label : undefined,
  }));

  const pick = (key: string) => {
    onPick(key);
    const row = rowRefs.current.get(key);
    row?.scrollIntoView({ block: "nearest" });
    // The mapping opens where the button is, so the reader sees the two together.
    row?.querySelector<HTMLButtonElement>(".select-trigger")?.click();
  };

  // What the pad is doing right now (#375). The loop runs only while this panel is on the page
  // and the editor is in front: it stops on blur, when the page is hidden, and when the panel
  // goes away. The device on the page says which pad to read when several are connected.
  const pad = usePadInput({ match: devicePadMatch(detail.info) });
  const planRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<HTMLDivElement | null>(null);
  // What each row does, in the words the schema uses. The painter needs it: a button that is
  // mapped to another button lights when that one is held.
  const mapRows = useMemo<PadMapRow[]>(
    () =>
      rows.map((p) => {
        const held = plain(known(values, p.key));
        return { key: p.key, target: held === null ? null : String(held) };
      }),
    [rows, values],
  );
  const mapRowsRef = useRef(mapRows);
  // The last frame the painter was given. A held key must not go stale: when the map or the
  // profile changes, the same frame is painted again, so the key it lights is the new one.
  const lastFrameRef = useRef<PadFrame | null>(null);
  useEffect(() => {
    mapRowsRef.current = mapRows;
  }, [mapRows]);

  /**
   * Writes the pad's state into the drawing: the keys that are held, the travel of the two
   * triggers and where the two sticks stand. It writes into the page's own elements, so the
   * drawing is right in the frame the press was read in.
   */
  const paint = useCallback((frame: PadFrame) => {
    lastFrameRef.current = frame;
    const plan = planRef.current;
    const map = mapRef.current;
    if (!plan || !map) return;
    const lit = litKeys(frame, mapRowsRef.current);

    for (const el of plan.querySelectorAll<SVGGElement>("[data-pad-key]")) {
      const key = el.getAttribute("data-pad-key");
      el.setAttribute("data-pressed", key !== null && lit.has(key) ? "true" : "false");
    }
    for (const el of map.querySelectorAll<HTMLElement>("[data-key]")) {
      const key = el.getAttribute("data-key");
      el.setAttribute("data-pressed", key !== null && lit.has(key) ? "true" : "false");
    }

    // The two triggers: the slot fills with the travel, and the number counts it.
    for (const side of ["lt", "rt"] as const) {
      const slot = plan.querySelector<SVGRectElement>(`[data-pad-slot="${side}"]`);
      const fill = plan.querySelector<SVGRectElement>(`[data-pad-travel="${side}"]`);
      const number = plan.querySelector<SVGTextElement>(`[data-pad-value="${side}"]`);
      if (!slot || !fill || !number) continue;
      const travel = (side === "lt" ? frame.lt : frame.rt) / TRIGGER_SCALE;
      const x = Number(slot.getAttribute("x"));
      const width = Number(slot.getAttribute("width"));
      fill.setAttribute("width", String(width * travel));
      fill.setAttribute("x", String(side === "rt" ? x + width * (1 - travel) : x));
      number.textContent = String(side === "lt" ? frame.lt : frame.rt).padStart(3, "0");
    }

    // The two sticks: the dot stands where the stick stands, inside its well.
    for (const dot of plan.querySelectorAll<SVGCircleElement>("[data-pad-dot]")) {
      const which = dot.getAttribute("data-pad-dot");
      const cap = dot.parentElement?.querySelector(".pad-btn");
      if (!which || !cap) continue;
      const cx = Number(cap.getAttribute("cx"));
      const cy = Number(cap.getAttribute("cy"));
      const sx = which === "l" ? frame.lx : frame.rx;
      const sy = which === "l" ? frame.ly : frame.ry;
      dot.setAttribute("cx", String(cx + (sx / STICK_SCALE) * PAD_STICK_TRAVEL));
      dot.setAttribute("cy", String(cy + (sy / STICK_SCALE) * PAD_STICK_TRAVEL));
    }
    for (const axis of ["lx", "ly", "rx", "ry"] as const) {
      const number = plan.querySelector<HTMLElement>(`[data-pad-stick="${axis}"]`);
      if (!number) continue;
      number.textContent = `${frame[axis] < 0 ? "" : "+"}${frame[axis]}`;
    }

    plan.setAttribute("data-live", frame.connected ? "true" : "false");
  }, []);

  const register = pad.register;
  useEffect(() => register(paint), [register, paint]);

  // A held key does not go stale: when the map or the profile changes, the frame that is held
  // is painted again at once, so the light moves to the key the button now is.
  useEffect(() => {
    const last = lastFrameRef.current;
    if (last) paint(last);
  }, [mapRows, paint]);

  // The strip's second line: why no key is lit, or which pad is being shown.
  const padNote = pad.unreadable ? PAD_UNREADABLE_WORD : pad.showing;

  return (
    <Panel title={detail.info.name}>
      {/* What the pad is doing: three lines held in every state, so nothing below it moves. */}
      <div className="pad-state" data-state={pad.status} role="status">
        <span className="pad-state-dot" aria-hidden="true" />
        <span className="pad-state-line">
          <span className="pad-state-head">
            <span className="pad-state-text">{PAD_STATUS_WORDS[pad.status]}</span>
            {PAD_STUB_BUILT && pad.simulated && (
              <span className="pad-badge" data-on="true">
                <Chip>Simulated pad</Chip>
              </span>
            )}
          </span>
          <span className="pad-state-note" hidden={padNote === null}>
            {padNote ?? ""}
          </span>
        </span>
      </div>
      <div className="pad-grid">
        <div className="pad-plan-wrap" ref={planRef}>
          <ControllerOutline buttons={planButtons} selected={picked} onPick={pick} />
        </div>
        <div className="pad-map" ref={mapRef}>
          {rows.map((p) => {
            const options =
              p.kind.type === "choice"
                ? p.kind.options.map((o) => ({ value: o, label: choiceWord(o) }))
                : [];
            const held = plain(known(values, p.key));
            const current = held === null ? null : String(held);
            const fixed = readOnly.includes(p.key);
            return (
              <div
                key={p.key}
                className="pad-map-row"
                data-key={p.key}
                data-read={current === null ? "false" : "true"}
                data-on={picked === p.key ? "true" : "false"}
                ref={(el) => {
                  if (el) rowRefs.current.set(p.key, el);
                  else rowRefs.current.delete(p.key);
                }}
              >
                <span className="pad-map-label">{plainLabel(p.label)}</span>
                {current === null ? (
                  <UnreadValue label={`${plainLabel(p.label)} mapping`} />
                ) : fixed ? (
                  <span className="read-only">{choiceWord(current)}</span>
                ) : (
                  <Select
                    label={`${plainLabel(p.label)} mapping`}
                    value={current}
                    options={options}
                    disabled={disabled}
                    onChange={(v) => onChange(p.key, { type: "choice", value: v })}
                  />
                )}
                {/* The slot the word "pressed" stands in: it is always there, so the row never moves. */}
                <span className="pad-map-pressed">pressed</span>
              </div>
            );
          })}
        </div>
      </div>
    </Panel>
  );
}
