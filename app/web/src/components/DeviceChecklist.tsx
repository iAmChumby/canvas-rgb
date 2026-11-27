"use client";

import { useCallback, useState, type ReactNode } from "react";
import { ArrowClockwise, Check, Question, Warning } from "@phosphor-icons/react";
import { Button, ButtonLink } from "@/components/Button";
import { Panel } from "@/components/Panel";
import { getSnap } from "@/lib/store";
import { SUPPORTED_DEVICES } from "@/lib/supportedDevices";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { FansStatus } from "@/proto/FansStatus";
import type { Status } from "@/proto/Status";

const DEVICE_SUPPORT =
  "https://github.com/iAmChumby/canvas-rgb/blob/main/.github/ISSUE_TEMPLATE/device_support.yml";

type Verdict = "OK" | "Problem" | "Unknown";

interface Row {
  label: string;
  verdict: Verdict;
  words: ReactNode;
  next: string;
  /** The note's own "Ignore if" line, when there is one. */
  ignoreIf?: string;
  detail?: ReactNode;
}

function verdictIcon(verdict: Verdict) {
  if (verdict === "OK") return <Check size={15} weight="bold" aria-hidden="true" />;
  if (verdict === "Problem") return <Warning size={15} weight="fill" aria-hidden="true" />;
  return <Question size={15} aria-hidden="true" />;
}

/**
 * "Why isn't my device here?" (#580): a fixed checklist, read once on open and again only when
 * the person presses Try again. It never follows the status poll, so a row cannot change under a
 * reading finger. Every row says its verdict in words and gives exactly one next step.
 */
interface Snapshot {
  status: Status | null;
  devices: DeviceInfo[] | null;
  fans: FansStatus | null;
}

/** One read of the state, taken on open and on Try again, never on the status poll. */
function snapshot(): Snapshot {
  const snap = getSnap();
  return { status: snap.status, devices: snap.devices, fans: snap.fans };
}

export function DeviceChecklist() {
  // The state initializer is the open: the checklist holds what it saw until the person asks
  // again. No effect and no subscription, so a 1 s status poll cannot move a row.
  const [seen, setSeen] = useState<Snapshot>(snapshot);
  const run = useCallback(() => setSeen(snapshot()), []);
  const { status, devices, fans } = seen;

  const rows: Row[] = [];
  const openrgb = status?.openrgb ?? null;
  const openRgbFault = status?.faults?.find((f) => f.kind === "open_rgb_down") ?? null;
  rows.push(
    openrgb === "connected"
      ? { label: "OpenRGB link", verdict: "OK", words: "OpenRGB is connected.", next: "Nothing needed here." }
      : openrgb === "down"
        ? {
            label: "OpenRGB link",
            verdict: "Problem",
            words: openRgbFault?.cause ?? "OpenRGB is not reachable, so its lights are missing.",
            next: openRgbFault?.fix ?? "Start OpenRGB, then try again.",
            ignoreIf: openRgbFault?.ignore_if ?? undefined,
          }
        : {
            label: "OpenRGB link",
            verdict: "Unknown",
            words: "The link has not said whether OpenRGB is up.",
            next: "Wait for the status to arrive, then try again.",
          },
  );

  const signal = status?.conflicts?.find((c) => c.kind === "signal_rgb");
  rows.push(
    status === null
      ? {
          label: "SignalRGB",
          verdict: "Unknown",
          words: "The status has not arrived, so SignalRGB has not been checked.",
          next: "Try again once the status arrives.",
        }
      : signal
        ? {
            label: "SignalRGB",
            verdict: "Problem",
            words: (
              <>
                SignalRGB is running as <span className="fig">SignalRgb.exe</span> or{" "}
                <span className="fig">SignalRgb.Service</span>, and will fight canvas-rgb for devices.
              </>
            ),
            next: "Close SignalRGB, then try again.",
          }
        : {
            label: "SignalRGB",
            verdict: "OK",
            words: "SignalRgb.exe and SignalRgb.Service were not seen.",
            next: "Nothing needed here.",
          },
  );

  const chipProblem = status?.faults?.find((f) => f.kind === "fan_chip") ?? null;
  rows.push(
    chipProblem
      ? {
          // The daemon only sends this fault after an observed bad chip, so it outranks a stale
          // or missing fans cache.
          label: "Fan chip and PawnIO",
          verdict: "Problem",
          words: chipProblem.cause ?? chipProblem.affected,
          next: chipProblem.fix,
          ignoreIf: chipProblem.ignore_if ?? undefined,
        }
      : !fans?.chip_checked
        ? {
            label: "Fan chip and PawnIO",
            verdict: "Unknown",
            words: "The fan service has not said whether it checked the chip.",
            next: "Try again once the fan service answers.",
          }
        : {
            label: "Fan chip and PawnIO",
            verdict: "OK",
            words: "The fan chip answered through PawnIO.",
            next: "Nothing needed here.",
          },
  );

  rows.push({
    label: "Devices canvas-rgb drives itself",
    verdict: connectedCount(devices) > 0 ? "OK" : "Unknown",
    words: "These adapters are built in; a connected one is marked below.",
    next: "Compare the names with the device you expected.",
    detail: (
      <ul className="help-check-devices">
        {SUPPORTED_DEVICES.map((device) => {
          const connected = Array.isArray(devices) && devices.some((d) =>
            d.id.toLowerCase().includes(`${device.vid.toLowerCase()}:${device.pid.toLowerCase()}`),
          );
          return (
            <li key={`${device.name}:${device.vid}:${device.pid}`} data-connected={connected ? "true" : "false"}>
              <span>{device.name}</span>
              <span className="fig">
                {device.vid}:{device.pid}
              </span>
              <span className="help-check-connected">{connected ? "connected" : "not connected"}</span>
            </li>
          );
        })}
      </ul>
    ),
  });

  rows.push({
    label: "Everything else comes through OpenRGB",
    verdict: "Unknown",
    words: "If OpenRGB lists it and canvas-rgb does not, report it with the support template.",
    next: "Open the device support template and attach the system report from Help & About.",
    detail: (
      <ButtonLink href={DEVICE_SUPPORT} external size="sm" variant="ghost">
        Device support template
      </ButtonLink>
    ),
  });

  return (
    <Panel title="Why isn't my device here?" className="help-checklist">
      <ol className="help-check">
        {rows.map((row) => (
          <li key={row.label} data-verdict={row.verdict}>
            <span className="help-check-icon" aria-hidden="true">
              {verdictIcon(row.verdict)}
            </span>
            <div className="help-check-body">
              <div className="help-check-head">
                <h3>{row.label}</h3>
                <span className="help-check-verdict">{row.verdict}</span>
              </div>
              <p>{row.words}</p>
              <p className="mute">
                <span className="help-check-label">Fix:</span> {row.next}
              </p>
              {row.ignoreIf && (
                <p className="mute">
                  <span className="help-check-label">Ignore if:</span> {row.ignoreIf}
                </p>
              )}
              {row.detail}
            </div>
          </li>
        ))}
      </ol>
      <div className="help-check-actions">
        <Button size="sm" icon={<ArrowClockwise size={14} />} onClick={run}>
          Try again
        </Button>
      </div>
    </Panel>
  );
}

function connectedCount(devices: DeviceInfo[] | null): number {
  return Array.isArray(devices) ? devices.filter((d) => d.connected).length : 0;
}
