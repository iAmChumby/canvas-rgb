import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { applyBulk } from "@/lib/fanBulk";
import { writeFanMode } from "@/lib/fanWrites";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";

// A header takes one write at a time and only the newest one waits (#110). The write that loses
// must say so, so a bulk run never reports a mode that was never sent as applied.

afterEach(() => vi.restoreAllMocks());

/** Replaces the PATCH with one the test releases by hand, and records what went out. */
function gatedPatch() {
  const sent: string[] = [];
  const gates: Array<{ release: () => void; fail: (e: Error) => void }> = [];
  vi.spyOn(api, "patchFanHeader").mockImplementation(
    (header: string, mode: FanMode) =>
      new Promise((resolve, reject) => {
        sent.push(`${header}:${mode.mode === "fixed" ? mode.percent : mode.mode}`);
        gates.push({ release: () => resolve({ ok: true, header, mode } as never), fail: reject });
      }),
  );
  return { sent, gates };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("writeFanMode", () => {
  it("drops the write a newer one replaced and tells it so", async () => {
    const { sent, gates } = gatedPatch();
    const errs: string[] = [];
    const first = writeFanMode("hdr_a", { mode: "fixed", percent: 30 }, () => errs.push("first"));
    const second = writeFanMode("hdr_a", { mode: "fixed", percent: 40 }, () => errs.push("second"));
    const third = writeFanMode("hdr_a", { mode: "fixed", percent: 55 }, () => errs.push("third"));
    await expect(second).resolves.toBe("superseded");
    gates[0]!.release();
    await expect(first).resolves.toBe("sent");
    await tick();
    gates[1]!.release();
    await expect(third).resolves.toBe("sent");
    expect(sent).toEqual(["hdr_a:30", "hdr_a:55"]);
    expect(errs).toEqual([]);
  });

  it("sends a refusal to the write it belongs to, not to the one that started the run", async () => {
    const { gates } = gatedPatch();
    const errs: string[] = [];
    const first = writeFanMode("hdr_b", { mode: "bios" }, () => errs.push("first"));
    const second = writeFanMode("hdr_b", { mode: "fixed", percent: 50 }, () => errs.push("second"));
    gates[0]!.release();
    await first;
    await tick();
    gates[1]!.fail(new Error("refused"));
    await expect(second).resolves.toBe("sent");
    expect(errs).toEqual(["second"]);
  });
});

describe("applyBulk against a header another edit is writing", () => {
  const header = (id: string): FanHeaderStatus =>
    ({ id, label: id, rpm: 900, pwm_percent: 30, mode: { mode: "bios" }, on_bios: false, override_reason: null, available: true }) as FanHeaderStatus;
  const fans = { headers: [header("hdr_c"), header("hdr_d")], floor_percent: 20 } as unknown as FansStatus;

  it("reports the superseded header skipped, not applied, and the others as they went", async () => {
    const { sent, gates } = gatedPatch();
    // The editor has a write out on hdr_c.
    const editor = writeFanMode("hdr_c", { mode: "fixed", percent: 30 }, () => {});
    const run = applyBulk(fans.headers, { kind: "fixed", percent: 60 }, fans, writeFanMode);
    await tick();
    // The editor's next edit lands while the bulk write waits behind the first one.
    const edit = writeFanMode("hdr_c", { mode: "fixed", percent: 45 }, () => {});
    await tick();
    gates[0]!.release();
    await editor;
    await tick();
    for (const g of gates) g.release(); // a settled request releases again harmlessly
    const out = await run;
    await edit;

    // hdr_c got the editor's 30 and 45 and never the bulk 60; hdr_d got the bulk 60.
    expect([...sent].sort()).toEqual(["hdr_c:30", "hdr_c:45", "hdr_d:60"]);
    expect(out.map((o) => [o.id, o.state])).toEqual([
      ["hdr_c", "skipped"],
      ["hdr_d", "applied"],
    ]);
    expect(out[0]!.detail).toMatch(/Superseded by your edit/);
  });

  it("stops writing when the signal aborts and returns what was done", async () => {
    const { sent, gates } = gatedPatch();
    const ctl = new AbortController();
    const run = applyBulk(fans.headers, { kind: "bios" }, fans, writeFanMode, ctl.signal);
    await tick();
    ctl.abort();
    gates[0]!.release();
    const out = await run;
    expect(sent).toHaveLength(1);
    expect(out.map((o) => [o.id, o.state])).toEqual([["hdr_c", "applied"]]);
  });
});
