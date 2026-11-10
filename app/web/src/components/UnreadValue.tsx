"use client";

import { Tooltip } from "@/components/Tooltip";
import { NOT_READ_WORDS } from "@/lib/peripherals";

/**
 * A setting the device did not report (#276): a dash in the place of its control, with the tooltip
 * "not read from the device". It is a value that is unknown, so it offers nothing to change: a
 * default the device never held must not be written. `label` names the setting for a screen reader.
 */
export function UnreadValue({ label }: { label: string }) {
  return (
    <Tooltip label={NOT_READ_WORDS}>
      <span className="unread-value" data-role="unread-setting" role="img" aria-label={`${label}: ${NOT_READ_WORDS}`}>
        <span aria-hidden="true">—</span>
      </span>
    </Tooltip>
  );
}
