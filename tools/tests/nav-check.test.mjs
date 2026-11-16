// Regression test for #57: the one-line icon assertion in tools/nav-check.mjs used to skip a
// link with no .rail-icon, so a dock with a silent hole still passed. judge() is exported from
// the tool and tested here with a synthetic measurement: every link has an icon, then one is
// removed, and the check must fail on that link. Run with: node tools/tests/nav-check.test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { judge, PAGES } from "../nav-check.mjs";

const PAGE = { name: "devices", label: "Devices" };

function measurement() {
  return {
    vw: 390,
    docOverflow: 0,
    nav: { top: 800, bottom: 865, left: 0, right: 390, position: "fixed" },
    links: PAGES.map((p, i) => ({
      text: p.label,
      active: p.label === "Peripherals",
      left: i * 50, right: i * 50 + 48, top: 810, bottom: 866,
      hidden: p.label !== "Peripherals",
      labelScroll: 10, labelClient: 10, labelLeft: i * 50, labelRight: i * 50 + 10,
      fontPx: 14, display: "grid",
      iconCentre: 818, iconTop: 809, iconHeight: 18,
    })),
    lowest: 700, lowestName: "main", vh: 844,
  };
}

test("a full dock passes the one-line icon check", () => {
  assert.deepEqual(judge(measurement(), PAGE, 390), []);
});

test("a link with no .rail-icon is a failure, not a skipped link", () => {
  const m = measurement();
  m.links[2].iconCentre = null; // Devices
  const bad = judge(m, PAGE, 390);
  assert.ok(
    bad.some((b) => b.includes("Devices") && b.includes("no .rail-icon")),
    `expected a missing-icon failure, got: ${JSON.stringify(bad)}`,
  );
});
