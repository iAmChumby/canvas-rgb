"use client";

import { Fragment, useCallback, useEffect, useRef, useState, type CSSProperties, type ReactElement } from "react";
import {
  ArrowClockwise,
  Cpu,
  GraphicsCard,
  Keyboard,
  Lightbulb,
  Memory,
  Microphone,
  Mouse,
  PencilSimple,
  Plugs,
  SlidersHorizontal,
} from "@phosphor-icons/react";
import type { Icon } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { NameField } from "@/components/NameField";
import { Panel } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Toggle } from "@/components/Toggle";
import { InlineSkeleton } from "@/components/Skeleton";
import { Slider } from "@/components/Slider";
import { ErrorPanel, Details, LoadFrame } from "@/components/Loading";
import { DeviceListShape } from "@/components/LoadShapes";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshLights, retryDevices, useSnap } from "@/lib/store";
import type { LightPatch } from "@/proto/LightPatch";
import type { LightView } from "@/proto/LightView";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { WhiteBalance } from "@/proto/WhiteBalance";
import type { WhiteBalances } from "@/proto/WhiteBalances";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { IdentifyWalker } from "@/views/editor/IdentifyWalker";
import { PageTitle } from "@/shell/PageHeader";

const KIND_ICON: Record<DeviceKind, Icon> = {
  motherboard: Cpu,
  dram: Memory,
  keyboard: Keyboard,
  mouse: Mouse,
  microphone: Microphone,
  headset: Microphone,
  led_strip: Lightbulb,
  gpu: GraphicsCard,
  other: Plugs,
};

const KIND_LABEL: Record<DeviceKind, string> = {
  motherboard: "Motherboard",
  dram: "DRAM",
  keyboard: "Keyboard",
  mouse: "Mouse",
  microphone: "Microphone",
  headset: "Headset",
  led_strip: "LED strip",
  gpu: "GPU",
  other: "Other",
};

const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/**
 * The order the list is in: the devices that are not there first, then by kind - the board
 * first, then the memory, the card, the fans, the strips and the peripherals. Inside a group
 * the daemon's own order stands.
 */
const KIND_RANK: Record<DeviceKind, number> = {
  motherboard: 0,
  dram: 1,
  gpu: 2,
  other: 3,
  led_strip: 4,
  keyboard: 5,
  mouse: 5,
  microphone: 5,
  headset: 5,
};

function zoneShape(z: ZoneInfo): string {
  if (z.kind.type === "matrix") return `matrix ${z.kind.width} × ${z.kind.height}`;
  if (z.kind.type === "linear") return "linear";
  return "single";
}

/**
 * The one detail that tells two devices apart when the daemon gives them the same name: the
 * address in their location. Two ENE DRAM sticks both answer to "ENE DRAM"; their SMBus
 * addresses do not.
 */
function deviceTag(d: DeviceInfo): string | null {
  const hex = d.location.match(/0x[0-9a-fA-F]+/g)?.pop();
  return hex ?? null;
}

/**
 * A zone drawn as its LEDs: a row of lamps for a short strip, a lit bar for a long one, and the
 * matrix grid with filled cells where the map has an LED. Coloured by the device's accent.
 */
function ZoneStrip({ zone }: { zone: ZoneInfo }) {
  if (zone.kind.type === "matrix") {
    const { width: cols, height: rows, map } = zone.kind;
    const cells: ReactElement[] = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const lit = map[r * cols + c] !== null && map[r * cols + c] !== undefined;
        cells.push(
          <circle key={`${r}-${c}`} cx={c + 0.5} cy={r + 0.5} r={lit ? 0.36 : 0.12} fill="currentColor" opacity={lit ? 1 : 0.35} />,
        );
      }
    }
    return (
      <svg className="zone-strip-svg" viewBox={`0 0 ${cols} ${rows}`} preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${cols} by ${rows} matrix`}>
        {cells}
      </svg>
    );
  }
  // A single LED has no strip to draw: its one dot would sit alone in the middle of the row and read
  // as a stray mark. The count beside it says it is one LED.
  if (zone.kind.type === "single") {
    return <span className="zone-strip-svg" aria-hidden="true" />;
  }
  const n = Math.max(0, Math.min(zone.led_count, 240));
  if (n === 0) {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: no LEDs yet`}>
        <line x1="40" y1="11" x2="160" y2="11" stroke="currentColor" strokeWidth="1.5" strokeDasharray="4 4" opacity="0.45" />
      </svg>
    );
  }
  const step = 200 / n;
  // A short strip: one lamp per LED. A long one: the lamps run into each other, so the strip is
  // drawn as the lit bar it is, the way it looks on the desk.
  if (n > 24) {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${zone.led_count} LEDs`}>
        <rect x="0" y="4" width="200" height="14" rx="5" fill="currentColor" opacity="0.16" />
        <rect x="0" y="6.5" width="200" height="9" rx="4.5" fill="currentColor" opacity="0.5" />
        <rect x="0" y="9.5" width="200" height="3" rx="1.5" fill="currentColor" />
      </svg>
    );
  }
  const r = Math.min(4, Math.max(1.8, step * 0.3));
  const dots: ReactElement[] = [];
  for (let i = 0; i < n; i++) {
    dots.push(<circle key={i} cx={(i + 0.5) * step} cy={11} r={r} fill="currentColor" />);
  }
  return (
    <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${zone.led_count} LEDs`}>
      <rect x="0" y="4" width="200" height="14" rx="5" fill="currentColor" opacity="0.16" />
      {dots}
    </svg>
  );
}

interface WalkTarget {
  deviceId: string;
  zoneIndex: number;
}

const NEUTRAL: WhiteBalance = { red: 1, green: 1, blue: 1 };

const CHANNELS = [
  { key: "red", label: "Red" },
  { key: "green", label: "Green" },
  { key: "blue", label: "Blue" },
] as const;

type Channel = (typeof CHANNELS)[number]["key"];

/** What pure white becomes on a device with these gains. */
function swatchOf(wb: WhiteBalance): string {
  return `rgb(${Math.round(255 * wb.red)}, ${Math.round(255 * wb.green)}, ${Math.round(255 * wb.blue)})`;
}

function isNeutral(wb: WhiteBalance): boolean {
  return wb.red === 1 && wb.green === 1 && wb.blue === 1;
}

/**
 * One device's white balance (#50), opened under its row from the row's Balance key, like the
 * Size walker: one gain per LED channel, shown as 0..100 %. Reset is there only while the
 * device is not neutral.
 */
function WhiteBalanceSection({
  device,
  name,
  shown,
  saved,
  known,
  onDraft,
  onPut,
  onReset,
  onClose,
  onRetry,
  error,
}: {
  device: DeviceInfo;
  name: string;
  shown: WhiteBalance;
  saved: WhiteBalance;
  /** The daemon has answered with this device's gains. False: they are being read, and the rows
   *  are placeholders, never the neutral values standing in for them. */
  known: boolean;
  onDraft: (deviceId: string, wb: WhiteBalance) => void;
  onPut: (deviceId: string, wb: WhiteBalance) => Promise<void>;
  onReset: (deviceId: string) => Promise<void>;
  onClose: () => void;
  /** Reads the white balances again, when the first read failed. */
  onRetry: () => void;
  error: string | null;
}) {
  // The read failed and nothing is known: an error and a way to ask again, never a skeleton for ever.
  const readFailed = !known && error !== null;
  const commit = (channel: Channel) => (pct: number) => {
    void onPut(device.id, { ...shown, [channel]: pct / 100 });
  };
  const drag = (channel: Channel) => (pct: number) => {
    onDraft(device.id, { ...shown, [channel]: pct / 100 });
  };

  return (
    <div className="rd-wb" role="group" aria-label={`White balance channels for ${name}`}>
      <div className="rd-wb-head">
        <span
          className="wb-swatch"
          data-unknown={known ? undefined : readFailed ? "failed" : "true"}
          style={known ? { backgroundColor: swatchOf(shown) } : undefined}
          aria-hidden="true"
        />
        <span className="rd-wb-title">White balance</span>
        <span className="mute rd-wb-hint">
          Show White on every light, then lower the channel this one has too much of until it matches.
        </span>
        {known && !isNeutral(saved) && (
          <Button size="sm" variant="ghost" onClick={() => void onReset(device.id)}>
            Reset
          </Button>
        )}
        <Button size="sm" onClick={onClose}>
          Done
        </Button>
      </div>
      <div className="rd-wb-rows" aria-busy={known || readFailed ? undefined : true}>
        {readFailed && (
          <div className="rd-wb-failed">
            <span className="error-line" role="alert">
              The white balance could not be read. {error}
            </span>
            <Button size="sm" onClick={onRetry}>
              Try again
            </Button>
          </div>
        )}
        {CHANNELS.map(({ key, label }) => !known ? (
          readFailed ? null : (
            <div className="rd-wb-row" key={key} data-channel={key}>
              <span className="rd-wb-label">{label}</span>
              <InlineSkeleton width="100%" />
              <span className="fig rd-wb-fig">
                <InlineSkeleton width="3ch" />
              </span>
            </div>
          )
        ) : (
          <div className="rd-wb-row" key={key} data-channel={key}>
            <span className="rd-wb-label">{label}</span>
            <Slider
              label={`${label} gain for ${name}`}
              min={0}
              max={100}
              value={Math.round(shown[key] * 100)}
              onChange={drag(key)}
              onCommit={commit(key)}
            />
            <span className="fig rd-wb-fig">{Math.round(shown[key] * 100)}%</span>
          </div>
        ))}
      </div>
      {error && !readFailed && <p className="error-line">{error}</p>}
    </div>
  );
}

/**
 * One light's row: its name, what it is (a strip or a ring, where the hardware says linear), and
 * whether it runs the other way. Every change goes straight to the lights route and the list is
 * read back, so two windows never disagree about a light.
 */
function LightControls({ view, zone }: { view: LightView; zone: ZoneInfo }) {
  const light = view.light;
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(view.label);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const patch = async (body: LightPatch) => {
    setBusy(true);
    setError(null);
    try {
      await api.lights.patch(light.id, body);
      await refreshLights();
    } catch (e) {
      setError(messageOf(e));
      throw e;
    } finally {
      setBusy(false);
    }
  };

  const commitName = () => {
    const next = name.trim();
    setEditing(false);
    if (next === view.label) return;
    void patch({ name: next || null }).catch(() => setName(view.label));
  };

  // The hardware says whether a light can be a ring: only a line can be one.
  const linear = zone.kind.type === "linear";

  return (
    <div className="rd-light-row" data-busy={busy ? "true" : undefined}>
      <span className="mute rd-light-key">Light</span>
      {editing ? (
        <NameField
          value={name}
          label={`Name for ${view.label}`}
          maxLength={40}
          disabled={busy}
          onChange={setName}
          onEnter={commitName}
          onEscape={() => {
            setName(view.label);
            setEditing(false);
          }}
        />
      ) : (
        <Button
          variant="ghost"
          size="sm"
          className="rd-light-name"
          aria-label={`Rename ${view.label}`}
          disabled={busy}
          onClick={() => {
            setName(view.label);
            setEditing(true);
          }}
        >
          <PencilSimple size={13} aria-hidden="true" />
          <span className="one-line">{view.label}</span>
        </Button>
      )}
      {linear && (
        <Segmented<"strip" | "ring">
          label={`What ${view.label} is`}
          value={light.arrangement === "ring" ? "ring" : "strip"}
          options={[
            { value: "strip", label: "Strip" },
            { value: "ring", label: "Ring" },
          ]}
          onChange={(next) => void patch({ arrangement: next }).catch(() => undefined)}
        />
      )}
      <span className="rd-light-turn">
        <span className="mute">Runs the other way</span>
        <Toggle
          label="Runs the other way"
          checked={light.reversed}
          disabled={busy}
          onChange={(next) => void patch({ reversed: next }).catch(() => undefined)}
        />
      </span>
      {error && <span className="error-line rd-light-error">{error}</span>}
    </div>
  );
}

export function DevicesView() {
  const devices = useSnap((s) => s.devices);
  const devicesError = useSnap((s) => s.devicesError);
  const lights = useSnap((s) => s.lights);
  // How many lights the daemon has already counted: the placeholder draws as many cards.
  const counted = useSnap((s) => s.status?.devices_connected ?? 0);
  // How many devices the daemon has, connected or not. The status route knows the total, so the
  // placeholder draws as many cards as the list will show, from the very first load. A daemon
  // that does not report a total (an older one) leaves it out, and the number is null then.
  const total = useSnap((s) => s.status?.devices_total ?? null);
  // The length of the last list that came: the total for a daemon that does not report one.
  const listed = useSnap((s) => s.devicesTotal);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("devices"));
  const openrgb = useSnap((s) => s.status?.openrgb ?? "connected");
  // The first-run banner links here with ?walk=<device id>. That device's first resizable zone opens
  // in the walker until the user closes it or picks another zone.
  const [urlWalk] = useState<string | null>(() =>
    typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("walk"),
  );
  const [urlHandled, setUrlHandled] = useState(false);
  const [choice, setChoice] = useState<WalkTarget | null>(null);
  const fromUrl: WalkTarget | null = (() => {
    if (urlHandled || !urlWalk || !devices) return null;
    const d = devices.find((x) => x.id === urlWalk);
    // The first header that already reports LEDs: a header that reports none is the fixed
    // part of the board, not an addressable strip.
    const z = d?.zones.find((x) => x.resizable && x.led_count > 0) ?? d?.zones.find((x) => x.resizable);
    return d && z ? { deviceId: d.id, zoneIndex: z.index } : null;
  })();
  const walk = choice ?? fromUrl;
  const setWalk = (next: WalkTarget | null) => {
    setUrlHandled(true);
    setChoice(next);
  };

  const walkDevice = devices?.find((d) => d.id === walk?.deviceId) ?? null;
  const walkZone = walkDevice?.zones.find((z) => z.index === walk?.zoneIndex) ?? null;

  // The white balances, read once when the view mounts (#50); a device not listed is neutral.
  const [whiteBalances, setWhiteBalances] = useState<WhiteBalances | null>(null);
  const [wbDrafts, setWbDrafts] = useState<Record<string, WhiteBalance>>({});
  const [wbError, setWbError] = useState<string | null>(null);
  // The one device whose white balance is open under its row, if any.
  const [wbOpen, setWbOpen] = useState<string | null>(null);
  // Each read carries a number; only the newest read may answer (a retry supersedes a slow one).
  const wbRead = useRef(0);
  const readWb = useCallback(() => {
    const n = ++wbRead.current;
    api
      .getWhiteBalance()
      .then((wb) => {
        if (wbRead.current === n) setWhiteBalances(wb);
      })
      .catch((e: unknown) => {
        if (wbRead.current === n) setWbError(messageOf(e));
      });
  }, []);
  // "Try again" after a failed read: the error goes, the read is asked for again.
  const retryWb = () => {
    setWbError(null);
    readWb();
  };
  useEffect(() => {
    const reads = wbRead;
    readWb();
    return () => {
      reads.current++;
    };
  }, [readWb]);

  const wbDraft = (deviceId: string, wb: WhiteBalance) =>
    setWbDrafts((d) => ({ ...d, [deviceId]: wb }));

  const wbPut = async (deviceId: string, wb: WhiteBalance) => {
    try {
      setWhiteBalances(await api.putWhiteBalance(deviceId, wb));
      // A drag that started while this PUT was out keeps its draft: only the draft this PUT
      // sent is cleared.
      setWbDrafts((d) => {
        const cur = d[deviceId];
        if (cur && (cur.red !== wb.red || cur.green !== wb.green || cur.blue !== wb.blue)) return d;
        const next = { ...d };
        delete next[deviceId];
        return next;
      });
      setWbError(null);
    } catch (e) {
      // The control goes back to the value the daemon has: nothing fails silently.
      setWbDrafts((d) => {
        const next = { ...d };
        delete next[deviceId];
        return next;
      });
      setWbError(messageOf(e));
    }
  };

  const wbReset = async (deviceId: string) => {
    try {
      setWhiteBalances(await api.deleteWhiteBalance(deviceId));
      setWbDrafts((d) => {
        const next = { ...d };
        delete next[deviceId];
        return next;
      });
      setWbError(null);
    } catch (e) {
      setWbError(messageOf(e));
    }
  };

  // The list in the order it is read: what is not there first, then by kind.
  const sorted = [...(devices ?? [])].sort((a, b) => {
    if (a.connected !== b.connected) return a.connected ? 1 : -1;
    return KIND_RANK[a.kind] - KIND_RANK[b.kind];
  });

  // How many devices are there, and how many of them are lit: the strip says it in one line.
  const connectedCount = devices?.filter((d) => d.connected).length ?? 0;
  const litCount = (devices ?? [])
    .filter((d) => d.connected)
    .reduce((sum, d) => sum + d.zones.reduce((s, z) => s + z.led_count, 0), 0);

  return (
    <div className="rd-page rd-devices">
      <PageTitle
        title="Devices"
        contextBusy={devices === null && !devicesError}
        context={
          devices === null ? (
            // Not read yet: "0/0 connected" would be a count of nothing drawn as a fact.
            devicesError ? undefined : <InlineSkeleton width="24ch" />
          ) : (
            <>
              <b>
                {connectedCount}/{devices.length}
              </b>{" "}
              connected · {litCount} LEDs
            </>
          )
        }
      />

      {devicesError && devices !== null && (
        // The list is on screen but the last read failed: the cards stay and the page says they
        // may be old. The raw words are behind the disclosure.
        <div className="rule rule-warn" role="status">
          The device list may be out of date.
          <Details>{devicesError}</Details>
        </div>
      )}
      {openrgb !== "connected" && (
        <p className="rule rule-warn">
          OpenRGB is not connected.
        </p>
      )}

      <LoadFrame
        className="load-list"
        state={
          devices === null
            ? devicesError && !retrying
              ? "error"
              : "loading"
            : devices.length === 0
              ? // A rescan is running: the empty answer is the old one, so the frame waits.
                retrying
                ? "loading"
                : "empty"
              : "ready"
        }
        shape={
          // A known zero is a known zero: a machine with no devices has none, so the
          // placeholder draws no card. Only a number that is not known at all falls through
          // to the next one.
          <DeviceListShape cards={total ?? listed ?? (counted > 0 ? counted : 3)} />
        }
        label="Looking for devices"
        error={
          <ErrorPanel
            title="The devices could not be read"
            summary="canvas-rgb could not read the devices. The lights are unaffected."
            message={devicesError}
            onRetry={() => void retryDevices()}
          />
        }
        empty={
          <Panel>
            <EmptyState
              icon={<Memory size={22} />}
              title="No device found"
              action={
                <Button size="sm" icon={<ArrowClockwise size={16} />} busy={retrying} onClick={() => void retryDevices()}>
                  Try again
                </Button>
              }
            >
              OpenRGB reports no lights. Plug one in or start OpenRGB, then try again.
            </EmptyState>
          </Panel>
        }
      >
        <div className="rd-dev-list">
          {sorted.map((d) => {
            const Glyph = KIND_ICON[d.kind];
            const accent = ACCENTS[sorted.indexOf(d) % ACCENTS.length];
            const tag = deviceTag(d);
            const sameName = (devices ?? []).filter((x) => x.name === d.name).length > 1;
            // The name the row shows, so two devices of one model get two different labels.
            const shownName = `${d.name}${sameName && tag ? ` ${tag}` : ""}`;
            const sub = `${KIND_LABEL[d.kind]} · ${d.vendor} · ${d.adapter} #${d.adapter_index}${tag ? ` · ${tag}` : ""}`;
            // The zones of a device that has one are the device's own row; a device with
            // several gets a row of its own and a row for each of its zones.
            const one = d.zones.length === 1 ? d.zones[0] : null;
            const lightOf = (zoneIndex: number) =>
              lights?.find((l) => l.light.device_id === d.id && l.light.zone_index === zoneIndex);
            return (
              <section
                key={d.id}
                className="rd-dev"
                style={{ "--tone": `var(${accent})` } as CSSProperties}
                aria-label={d.name}
                data-testid="device-card"
              >
                <div className="rd-dev-row">
                  <span className="device-kind" aria-hidden="true">
                    <Glyph size={18} />
                  </span>
                  <span className="one-line rd-dev-name">
                    {d.name}
                    {sameName && tag ? ` ${tag}` : ""}
                  </span>
                  <span className="row-sub fig one-line rd-dev-sub" title={sub}>
                    {sub}
                  </span>
                  {one && <ZoneStrip zone={one} />}
                  {one && (
                    <span className="zone-strip-count">
                      <AnimatedNumber value={one.led_count} /> {one.led_count === 1 ? "LED" : "LEDs"}
                    </span>
                  )}
                  {(() => {
                    const wbKnown = whiteBalances !== null;
                    const wb = wbDrafts[d.id] ?? whiteBalances?.devices[d.id] ?? NEUTRAL;
                    const open = wbOpen === d.id;
                    return (
                      <Button
                        size="sm"
                        className="wb-key"
                        variant={open ? "primary" : "default"}
                        aria-expanded={open}
                        aria-label={`White balance for ${shownName}${wbKnown && !isNeutral(wb) ? " (set)" : ""}`}
                        icon={
                          <span
                            className="wb-swatch"
                            data-unknown={wbKnown ? undefined : wbError ? "failed" : "true"}
                            style={wbKnown ? { backgroundColor: swatchOf(wb) } : undefined}
                            aria-hidden="true"
                          />
                        }
                        onClick={() => setWbOpen(open ? null : d.id)}
                      >
                        <span className="wb-key-text">Balance</span>
                        <SlidersHorizontal size={14} className="wb-key-glyph" aria-hidden="true" />
                      </Button>
                    );
                  })()}
                  <span className={`state ${d.connected ? "" : "down-text"}`} style={{ "--tone": d.connected ? "var(--ok)" : "var(--down)" } as CSSProperties}>
                    <span className="state-dot" aria-hidden="true" />
                    {d.connected ? "connected" : "disconnected"}
                  </span>
                  {/* A device with one zone has no row of zones under it, so the key that sizes
                      the zone sits in the device's own row, in the same column as the zone rows'. */}
                  <span className="zone-strip-size">
                    {one?.resizable && (
                      <Button
                        size="sm"
                        variant={walk?.deviceId === d.id && walk.zoneIndex === one.index ? "primary" : "default"}
                        onClick={() => setWalk({ deviceId: d.id, zoneIndex: one.index })}
                      >
                        Size
                      </Button>
                    )}
                  </span>
                </div>

                {one && lightOf(one.index) && (
                  <LightControls view={lightOf(one.index)!} zone={one} />
                )}

                {d.zones.length === 0 && (
                  <div className="zone-empty rd-zone-empty">
                    <EmptyState
                      icon={<Memory size={20} />}
                      title={`${d.name} reports no zones`}
                      action={
                        <Button size="sm" icon={<ArrowClockwise size={16} />} busy={retrying} onClick={() => void retryDevices()}>
                          Try again
                        </Button>
                      }
                    >
                      OpenRGB lists this device but no zones for it yet. Try again once it is lit.
                    </EmptyState>
                  </div>
                )}

                {d.zones.length > 1 &&
                  d.zones.map((z) => (
                    <Fragment key={z.index}>
                      <div className="rd-zone">
                        <span className="zone-strip-name">
                          <span className="one-line">{z.name}</span>
                          <span className="mute one-line rd-zone-shape">{zoneShape(z)}</span>
                        </span>
                        <ZoneStrip zone={z} />
                        <span className="zone-strip-count">
                          <AnimatedNumber value={z.led_count} /> {z.led_count === 1 ? "LED" : "LEDs"}
                          {z.resizable && <span className="mute"> · {z.min_leds}–{z.max_leds}</span>}
                        </span>
                        <span className="zone-strip-size">
                          {z.resizable && (
                            <Button
                              size="sm"
                              variant={walk?.deviceId === d.id && walk.zoneIndex === z.index ? "primary" : "default"}
                              onClick={() => setWalk({ deviceId: d.id, zoneIndex: z.index })}
                            >
                              Size
                            </Button>
                          )}
                        </span>
                      </div>
                      {lightOf(z.index) && <LightControls view={lightOf(z.index)!} zone={z} />}
                    </Fragment>
                  ))}

                {/* The walker opens under the row whose Size was pressed, not in a column of
                    its own (R7). */}
                {walkDevice?.id === d.id && walkZone && (
                  <div className="rd-walk">
                    <IdentifyWalker
                      key={`${walkDevice.id}#${walkZone.index}`}
                      device={walkDevice}
                      zone={walkZone}
                      onClose={() => setWalk(null)}
                    />
                  </div>
                )}

                {wbOpen === d.id && (
                  <WhiteBalanceSection
                    device={d}
                    name={shownName}
                    shown={wbDrafts[d.id] ?? whiteBalances?.devices[d.id] ?? NEUTRAL}
                    saved={whiteBalances?.devices[d.id] ?? NEUTRAL}
                    known={whiteBalances !== null}
                    onDraft={wbDraft}
                    onPut={wbPut}
                    onReset={wbReset}
                    onClose={() => setWbOpen(null)}
                    onRetry={retryWb}
                    error={wbError}
                  />
                )}
              </section>
            );
          })}
        </div>
      </LoadFrame>
    </div>
  );
}

