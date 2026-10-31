"use client";

import type { CSSProperties, ReactNode } from "react";
import { Skeleton } from "@/components/Skeleton";

/**
 * The shapes the loading frame holds, one per kind of content. Each is drawn from the same
 * classes as the content it stands for (`panel`, `peripheral-row`, `layout-grid`, ...), and each
 * panel is as tall as the real panel measured at 1440 and 390 px, so the page does not move when
 * the data lands. A page whose real content differs from the one measured (a device with other
 * settings, a longer list) grows or shrinks at the bottom of the frame, where nothing stands
 * under it.
 */

/** The height of a box on a wide page and on a phone, in px: what the real one measured. */
type Size = readonly [wide: number, narrow: number];

/** A panel with a title bar, like `Panel`. `size` is the height of the panel it stands for. */
function SkelPanel({
  title = true,
  size,
  row,
  children,
}: {
  title?: boolean;
  size?: Size;
  row?: Size;
  children?: ReactNode;
}) {
  const vars: Record<string, string> = {};
  if (size) {
    vars["--sh-w"] = `${size[0]}px`;
    vars["--sh-n"] = `${size[1]}px`;
  }
  if (row) {
    vars["--row-w"] = `${row[0]}px`;
    vars["--row-n"] = `${row[1]}px`;
  }
  return (
    <section
      className={size ? "panel skel-sized" : "panel"}
      style={Object.keys(vars).length > 0 ? (vars as CSSProperties) : undefined}
    >
      {title && (
        <div className="panel-head">
          <Skeleton className="skel-title" width={96} />
        </div>
      )}
      <div className="skel-body">{children}</div>
    </section>
  );
}

/** One row of a panel: its name on one side, its control on the other. */
function SkelProp({ label = 150, control = 160 }: { label?: number; control?: number | string }) {
  return (
    <div className="skel-prop">
      <Skeleton width={label} height={14} />
      <Skeleton width={control} height={32} radius={8} />
    </div>
  );
}

/**
 * A panel of `rows` rows, in a box as tall as the panel it stands for. `row` is the height of one
 * row on a wide page and on a phone (52 and 66 px unless the real rows measured otherwise).
 */
function SkelRows({
  rows,
  size,
  chips = false,
  row,
}: {
  rows: number;
  size: Size;
  chips?: boolean;
  row?: Size;
}) {
  return (
    <SkelPanel size={size} row={row}>
      {Array.from({ length: rows }, (_, i) => (
        <SkelProp key={i} label={110 + ((i * 41) % 70)} control={i % 2 === 0 ? 150 : 110} />
      ))}
      {chips && (
        <div className="skel-chips">
          {Array.from({ length: 6 }, (_, i) => (
            <Skeleton key={i} width={72} height={32} radius={8} />
          ))}
        </div>
      )}
    </SkelPanel>
  );
}

/** A list of `count` items that share the height of their panel, each with `lines` lines of text. */
function SkelBands({ count, lines }: { count: number; lines: number }) {
  return (
    <div className="skel-rows">
      {Array.from({ length: count }, (_, i) => (
        <div className="skel-row" key={i}>
          {Array.from({ length: lines }, (_, j) => (
            <Skeleton
              key={j}
              width={j === 0 ? `${32 + ((i * 13) % 22)}%` : `${54 + ((i * 17) % 30)}%`}
              height={14}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

/** The list of peripherals: a row per device. */
export function PeripheralListShape({ rows = 2 }: { rows?: number }) {
  return (
    <div className="peripheral-list">
      {Array.from({ length: rows }, (_, i) => (
        <div className="peripheral-row" key={i}>
          <Skeleton width={34} height={34} radius={8} style={{ flex: "none" }} />
          <span className="peripheral-text">
            <Skeleton width="58%" height={16} />
            <Skeleton width="80%" height={14} style={{ marginTop: 4 }} />
          </span>
        </div>
      ))}
    </div>
  );
}

/** The head and the access panel every peripheral page opens with. */
function PeripheralTop({ headSize }: { headSize: Size }) {
  return (
    <>
      <SkelPanel title={false} size={headSize}>
        <div className="peripheral-head">
          <Skeleton width={44} height={44} radius={10} style={{ flex: "none" }} />
          <div className="peripheral-head-text">
            <Skeleton width="42%" height={24} />
            <Skeleton width="68%" height={14} style={{ marginTop: 6 }} />
          </div>
          <Skeleton className="skel-head-status" width={92} height={14} />
        </div>
      </SkelPanel>
      <SkelPanel size={[166.8, 187.1]}>
        <div className="access-list">
          <div className="access-row">
            <Skeleton width={190} height={16} />
            <Skeleton width={40} height={24} radius={12} />
            <Skeleton className="access-note" width="64%" height={14} />
          </div>
          <div className="access-row">
            <Skeleton width={170} height={16} />
            <Skeleton width={40} height={24} radius={12} />
            <Skeleton className="access-note" width="40%" height={14} />
          </div>
        </div>
      </SkelPanel>
    </>
  );
}

/** The save bar under a peripheral's panels. It is out of the flow on a phone. */
function SkelSave({ height = 47.8 }: { height?: number }) {
  return (
    <div className="layout-save skel-save">
      <div className="savebar" style={{ height }}>
        <Skeleton width={120} height={14} />
        <Skeleton width={150} height={30} radius={8} />
      </div>
    </div>
  );
}

/** One peripheral's page, by kind of device (measured on the Attack Shark X6 and the 8BitDo Ultimate 2). */
export function PeripheralDetailShape({ kind = "mouse" }: { kind?: "mouse" | "game_controller" | "keyboard" }) {
  if (kind === "game_controller") {
    return (
      <div className="stack">
        <PeripheralTop headSize={[79.3, 192.6]} />
        <SkelRows rows={4} size={[270.3, 329.4]} />
        <SkelPanel size={[921.6, 1341.7]}>
          <div className="skel-pad">
            <Skeleton width="100%" height="100%" radius={8} />
          </div>
        </SkelPanel>
        <SkelRows rows={10} size={[582.3, 865.3]} />
        <SkelPanel size={[84, 105.8]}>
          <Skeleton width="70%" height={14} />
        </SkelPanel>
        <SkelSave />
      </div>
    );
  }
  return (
    <div className="stack">
      <PeripheralTop headSize={[79.3, 167.6]} />
      <SkelRows rows={1} size={[171.3, 225.3]} chips />
      <SkelRows rows={2} size={[166.3, 222.8]} />
      <SkelRows rows={4} size={[270.3, 384.3]} />
      <SkelRows rows={2} size={[166.3, 226.8]} />
      <SkelRows rows={6} size={[374.3, 463.5]} />
      <SkelRows rows={1} size={[114.3, 128.3]} />
      <SkelSave />
    </div>
  );
}

/** A numeric display's card: the preview readout on the left, the controls on the right. */
export function DisplayShape() {
  return (
    <SkelPanel size={[384.6, 615.8]}>
      <div className="display-notice-slot" />
      <div className="display-card">
        <div className="display-readout">
          <Skeleton width={70} height={14} />
          <Skeleton width="100%" height={96} radius={8} style={{ marginTop: 12 }} />
          <Skeleton width={110} height={14} style={{ marginTop: 12 }} />
        </div>
        <div className="display-controls">
          <SkelProp label={80} control={180} />
          <SkelProp label={60} control={110} />
          <SkelProp label={80} control={110} />
          <SkelProp label={80} control={110} />
        </div>
      </div>
    </SkelPanel>
  );
}

/** The desk canvas on the left, and the panels beside it (or under it on a phone). */
export function LayoutShape() {
  return (
    <div className="layout-grid">
      <div className="layout-stage-col stack">
        <Skeleton className="skel-stage" />
        <SkelRows rows={2} size={[169.3, 292.3]} />
      </div>
      <div className="stack">
        <SkelPanel size={[155.1, 144]} />
        <SkelPanel size={[910, 910]}>
          <SkelBands count={7} lines={2} />
        </SkelPanel>
        <SkelPanel size={[306.9, 306.9]} />
        <SkelSave height={58} />
      </div>
    </div>
  );
}

/** The gallery of effect cards. */
export function EffectGalleryShape({ cards = 12 }: { cards?: number }) {
  return (
    <div className="effect-gallery">
      {Array.from({ length: cards }, (_, i) => (
        <div className="effect-card effect-card-skeleton" key={i}>
          <Skeleton className="skeleton-preview" />
          <span className="effect-card-meta">
            <Skeleton className="skeleton-line" height={21.8} width="55%" />
          </span>
        </div>
      ))}
    </div>
  );
}

/** One effect's page: the preview with its name and button, its presets, and its properties. */
export function EffectDetailShape() {
  return (
    <div className="stack">
      <SkelPanel title={false} size={[425.3, 411.6]}>
        <div className="stack" style={{ gap: 12 }}>
          <Skeleton className="skel-preview" width="100%" radius={8} />
          <div className="row-wrap" style={{ justifyContent: "space-between" }}>
            <Skeleton width={160} height={28} />
            <Skeleton width={110} height={32} radius={8} />
          </div>
          <Skeleton width="86%" height={14} />
        </div>
      </SkelPanel>
      <SkelPanel size={[126.3, 126.3]}>
        <Skeleton width={120} height={26} radius={8} />
      </SkelPanel>
      <SkelRows rows={6} size={[374.3, 454.5]} />
    </div>
  );
}

/** The fans page under its head: profiles, the headers and temperatures, and the header editor. */
export function FansShape() {
  return (
    <div className="stack">
      <SkelPanel size={[162, 191.8]}>
        <SkelProp label={130} control={220} />
      </SkelPanel>
      <div className="fans-grid">
        <div className="stack">
          <SkelPanel size={[667.1, 1646.5]}>
            <SkelBands count={6} lines={2} />
          </SkelPanel>
          <SkelPanel size={[173.1, 173.1]}>
            <SkelBands count={3} lines={1} />
          </SkelPanel>
        </div>
        <div className="stack fans-editor-col">
          <SkelPanel size={[770.8, 674.1]}>
            <Skeleton width="100%" height={180} radius={8} />
            <SkelProp label={110} control={140} />
          </SkelPanel>
        </div>
      </div>
    </div>
  );
}

/** The settings page under its head: two columns of panels, one column on a phone. */
export function SettingsShape() {
  return (
    <div className="settings-grid">
      <div className="stack">
        <SkelRows rows={2} size={[169.3, 239]} row={[46, 46]} />
        <SkelRows rows={3} size={[207.3, 237]} row={[46, 46]} />
      </div>
      <div className="stack">
        <SkelRows rows={1} size={[120.3, 140.6]} row={[46, 46]} />
        <SkelRows rows={2} size={[159.3, 159.3]} row={[46, 46]} />
        <SkelRows rows={2} size={[159.3, 189]} row={[46, 46]} />
        <SkelPanel size={[179.1, 179.1]}>
          <Skeleton width="80%" height={14} />
        </SkelPanel>
        <SkelRows rows={3} size={[294.6, 294.6]} row={[46, 46]} />
      </div>
    </div>
  );
}

/** The devices list: a card per device with its zone strips. */
export function DeviceListShape({ cards = 3 }: { cards?: number }) {
  return (
    <>
      {Array.from({ length: cards }, (_, i) => (
        <section className="device device-skeleton" key={i}>
          <div className="device-head">
            <Skeleton className="skeleton-icon" />
            <span className="stack" style={{ gap: 4, minWidth: 0 }}>
              <Skeleton height={16} width="52%" />
              <Skeleton height={12} width="34%" />
            </span>
          </div>
          <div className="zone-strip">
            <Skeleton height={14} width="28%" />
            <Skeleton height={14} width="42%" />
            <Skeleton height={14} width="14%" />
          </div>
        </section>
      ))}
    </>
  );
}
