"use client";
import { ErrorLine } from "@/components/ErrorState";

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
import { LedOrderControl } from "@/components/LedOrderControl";
import { directionPatch, effectiveFirstLed, identifyLed } from "@/lib/ledOrder";
import { Panel } from "@/components/Panel";
import { ShapePicker, shapesFor } from "@/components/ShapePicker";
import { Toggle } from "@/components/Toggle";
import { InlineSkeleton } from "@/components/Skeleton";
import { Slider } from "@/components/Slider";
import { ErrorPanel, Details, LoadFrame } from "@/components/Loading";
import { DeviceListShape } from "@/components/LoadShapes";
import { api } from "@/lib/endpoints";
import { errorText } from "@/lib/errors";
import { refreshLights, retryDevices, useSnap } from "@/lib/store";
import type { LightPatch } from "@/proto/LightPatch";
import type { LightView } from "@/proto/LightView";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { WhiteBalance } from "@/proto/WhiteBalance";
import type { WhiteBalances } from "@/proto/WhiteBalances";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { ChainRow } from "@/views/chain/ChainRow";
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

/** The count cell, in the same place on every row: a zone with no LEDs is "Not sized", never "0 LEDs". */
function ZoneCount({ zone }: { zone: ZoneInfo }) {
  if (zone.led_count === 0) return <span className="mute">Not sized</span>;
  return (
    <>
      <AnimatedNumber value={zone.led_count} /> {zone.led_count === 1 ? "LED" : "LEDs"}
      {zone.resizable && <span className="mute"> · {zone.min_leds}–{zone.max_leds}</span>}
    </>
  );
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
 * One device's white balance (#50), opened under its row from the row's Balance key: one gain
 * per LED channel, shown as 0..100 %. Reset is there only while the device is not neutral.
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
            <ErrorLine role="alert">
              The white balance could not be read. {error}
            </ErrorLine>
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
      {error && !readFailed && <ErrorLine>{error}</ErrorLine>}
    </div>
  );
}

/**
 * One light's row: its name, its shape (a line or a ring, and a grid where the zone is one), and
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
      setError(errorText(e));
      throw e;
    } finally {
      setBusy(false);
    }
  };

  // The first LED in force (a reversed light with none starts on its last LED), the way the lights
  // read it. Identify addresses the zone, so the light's slice start is added to it.
  const first = effectiveFirstLed(light.first_led, light.reversed, view.led_count);
  const identifyFirst = (led: number) =>
    void api
      .identify(light.device_id, {
        zone: light.zone_index,
        led: identifyLed(light.leds, led),
        count: 1,
        stop: false,
      })
      .catch(() => undefined);
  const turn = (next: boolean) =>
    void patch(directionPatch(next, light.first_led, light.reversed, view.led_count)).catch(() => undefined);

  const commitName = () => {
    const next = name.trim();
    setEditing(false);
    if (next === view.label) return;
    void patch({ name: next || null }).catch(() => setName(view.label));
  };

  // The shapes this zone may be laid out as (a line, a ring, and a grid where the zone is one).
  // A single-LED zone offers none, a matrix also offers Grid (#471 fix pass).
  const shapes = shapesFor(zone.kind.type);

  return (
    <div className="rd-light-row" data-testid="light-row" data-busy={busy ? "true" : undefined}>
      {/* The label and the name share the name column; the shape (or an empty cell for a one-LED
          light) and the direction sit in their own columns, so they start at one x on every row. */}
      <span className="rd-light-who">
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
      </span>
      <span className="rd-shape" data-testid="light-shape">
        <ShapePicker
          label={`What ${view.label} is`}
          value={light.arrangement}
          shapes={shapes}
          disabled={busy}
          onChange={(next) => void patch({ arrangement: next }).catch(() => undefined)}
        />
      </span>

      <span className="rd-led-order" data-testid="light-led-order">
        <LedOrderControl
          ledCount={view.led_count}
          firstLed={first}
          reversed={light.reversed}
          ring={light.arrangement === "ring"}
          label={`LED order of ${view.label}`}
          onFirstLed={(led) => {
            void patch({ first_led: led })
              .then(() => identifyFirst(led))
              .catch(() => undefined);
          }}
          onReversed={turn}
          onIdentify={() => identifyFirst(first)}
        />
      </span>

      <span className="rd-light-turn">
        <span className="mute">Runs the other way</span>
        <Toggle
          label="Runs the other way"
          checked={light.reversed}
          disabled={busy}
          onChange={turn}
        />
      </span>
      {error && <ErrorLine className="rd-light-error">{error}</ErrorLine>}
    </div>
  );
}

export function DevicesView() {
  const devices = useSnap((s) => s.devices);
  const devicesError = useSnap((s) => s.devicesError);
  const lights = useSnap((s) => s.lights);
  // Every resizable header and what is plugged into it (0.15.0). A header with several parts is
  // one of these; its row is `ChainRow`, and the per-zone light controls are for the rest.
  const chains = useSnap((s) => s.chains);
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
        if (wbRead.current === n) setWbError(errorText(e));
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
      setWbError(errorText(e));
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
      setWbError(errorText(e));
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
            summary="canvas-rgb could not read the devices. Try the read again."
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
        <div className="rd-dev-list panel">
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
            // A resizable header the daemon lists is drawn by its chain row, whatever state it is
            // in: a set-up header has one light per part, which the per-zone controls cannot say.
            const chainOf = (zoneIndex: number) =>
              chains?.find((c) => c.device_id === d.id && c.zone_index === zoneIndex);
            return (
              <section
                key={d.id}
                className="rd-dev"
                style={{ "--tone": `var(${accent})` } as CSSProperties}
                aria-label={d.name}
                data-testid="device-card"
              >
                {(() => {
                  // Every row on this page is built from the same seven columns: kind, name and
                  // sub-line, LED bar, count, balance key, state and action. A header's own row
                  // carries its state and its Set up key inline, so it is no longer drawn twice
                  // (a zone row followed by a separate "ARGB header" row) as it was before #471.
                  const wbKnown = whiteBalances !== null;
                  const wb = wbDrafts[d.id] ?? whiteBalances?.devices[d.id] ?? NEUTRAL;
                  const open = wbOpen === d.id;
                  const chainOne = one ? chainOf(one.index) : null;
                  const balance = (
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
                  const connected = (
                    <span className={`state ${d.connected ? "" : "down-text"}`} style={{ "--tone": d.connected ? "var(--ok)" : "var(--down)" } as CSSProperties}>
                      <span className="state-dot" aria-hidden="true" />
                      {d.connected ? "connected" : "disconnected"}
                    </span>
                  );
                  const deviceLead = (
                    <>
                      <span className="device-kind" aria-hidden="true">
                        <Glyph size={18} />
                      </span>
                      <span className="rd-dev-who">
                        <span className="one-line rd-dev-name">{shownName}</span>
                        <span className="row-sub fig one-line rd-dev-sub" title={sub}>
                          {sub}
                        </span>
                      </span>
                      {one ? <ZoneStrip zone={one} /> : <span aria-hidden="true" />}
                      {one ? (
                        <span className="zone-strip-count">
                          <ZoneCount zone={one} />
                        </span>
                      ) : (
                        <span aria-hidden="true" />
                      )}
                      {balance}
                    </>
                  );
                  const zoneLead = (z: ZoneInfo) => (
                    <>
                      <span className="device-kind" aria-hidden="true">
                        <Plugs size={18} />
                      </span>
                      <span className="rd-dev-who">
                        <span className="one-line rd-dev-name">{z.name}</span>
                        <span className="mute one-line rd-dev-sub">ARGB header · {zoneShape(z)}</span>
                      </span>
                      <ZoneStrip zone={z} />
                      <span className="zone-strip-count">
                        <ZoneCount zone={z} />
                      </span>
                      <span aria-hidden="true" />
                    </>
                  );
                  return (
                    <>
                      {one && chainOne ? (
                        <ChainRow chain={chainOne} lead={deviceLead} deviceState={connected} />
                      ) : (
                        <div className="rd-dev-row">
                          {deviceLead}
                          {connected}
                          <span aria-hidden="true" />
                        </div>
                      )}

                      {one && !chainOne && lightOf(one.index) && (
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
                            {chainOf(z.index) ? (
                              <ChainRow chain={chainOf(z.index)!} lead={zoneLead(z)} deviceState={connected} />
                            ) : (
                              <div className="rd-zone">
                                {zoneLead(z)}
                                {connected}
                                <span aria-hidden="true" />
                              </div>
                            )}
                            {!chainOf(z.index) && lightOf(z.index) && (
                              <LightControls view={lightOf(z.index)!} zone={z} />
                            )}
                          </Fragment>
                        ))}
                    </>
                  );
                })()}

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