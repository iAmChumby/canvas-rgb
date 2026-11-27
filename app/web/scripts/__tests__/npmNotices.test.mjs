// The npm notices must not depend on which OS npm installed for (#578).
// npm sbom lists every optional platform package from the lock file, including
// variants whose LICENSE file is absent from node_modules on this machine.
// This fake SBOM contains both win32 and linux binaries; the planner has to
// exclude them from package metadata alone and produce identical output.

import { describe, expect, it } from "vitest";
import { buildNotices } from "../npm-notices-lib.mjs";

const WIN = "@img/sharp-libvips-win32-x64";
const LINUX = "@img/sharp-libvips-linux-x64";
const SWC = "@next/swc-linux-x64-gnu";

const component = (name, version = "1.2.3") => ({
  name,
  version,
  licenses: [{ license: { id: "MIT" } }],
  properties: [{ name: "cdx:npm:package:path", value: `node_modules/${name}` }],
});

const lock = {
  packages: {
    [`node_modules/${WIN}`]: { os: ["win32"], cpu: ["x64"], optional: true },
    [`node_modules/${LINUX}`]: { os: ["linux"], cpu: ["x64"], optional: true },
    [`node_modules/${SWC}`]: { os: ["linux"], cpu: ["x64"], optional: true },
    "node_modules/react": { license: "MIT" },
  },
};

// On Windows only the win32 variant is installed; on Linux only the linux
// variant is. A reader that fails on the other variant models the real
// `no LICENSE file in node_modules` failure from CI.
function planWith(installedVariant) {
  return buildNotices({
    components: [component(WIN), component(LINUX), component(SWC), component("react", "19.3.1")],
    lock,
    manifest: { dependencies: {} },
    accepted: new Set(["MIT"]),
    generatedAt: "2026-11-26T00:00:00.000Z",
    readPackage: (rel) => {
      if (rel === "node_modules/react") return { license: "MIT" };
      if (rel === `node_modules/${installedVariant}`) return { license: "MIT" };
      throw new Error(`${rel}: package is not installed on this platform`);
    },
    readLicense: () => ({ file: "LICENSE", text: "MIT TEST TEXT" }),
  });
}

describe("npm notice platform filtering", () => {
  it("gives the same notices whether the win32 or the linux variant is installed", () => {
    const onWindows = planWith(WIN);
    const onLinux = planWith(LINUX);
    expect(onWindows).toEqual(onLinux);
  });

  it("excludes every platform-gated binary by its package metadata, not by the host", () => {
    const result = planWith(WIN);
    expect(result.packages).toBe(1);
    expect(result.groups.flatMap((g) => g.packages.map((p) => p.name))).toEqual(["react"]);
    expect(result.excluded.map((e) => e.name)).toEqual(expect.arrayContaining([LINUX, WIN, SWC]));
    const reason = result.excluded.find((e) => e.name === WIN).reason;
    expect(planWith(LINUX).excluded.find((e) => e.name === WIN).reason).toBe(reason);
    expect(reason).toContain("platform-native package");
  });
});
