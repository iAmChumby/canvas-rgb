"use client";

import { Tooltip } from "@/components/Tooltip";
import { NOT_READ_WORDS } from "@/lib/peripherals";

/**
 * A setting the device did not report (#276): a dash in the place of its control, with the tooltip
 * "not read from the device". It is a value that is unknown, so it offers nothing to change: a
 * default the device never held must not be written.
 *
 * The dash takes keyboard focus, so the tooltip shows on focus as it does on hover. Its accessible
 * name ("LED colour: not read from the device") is its `aria-label` alone, read once: the tooltip
 * bubble is not tied to it with `aria-describedby`, which would say the same words again.
 * `label` names the setting.
 */
export function UnreadValue({ label }: { label: string }) {
  return (
    <Tooltip label={NOT_READ_WORDS}>
      <span
        className="unread-value"
        data-role="unread-setting"
        role="group"
        tabIndex={0}
        aria-label={`${label}: ${NOT_READ_WORDS}`}
      >
        <span aria-hidden="true">—</span>
      </span>
    </Tooltip>
  );
}
