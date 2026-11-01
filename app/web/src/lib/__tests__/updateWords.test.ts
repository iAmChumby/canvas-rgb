import { describe, expect, it } from "vitest";
import { installFailedSentence, stateLine } from "@/lib/updateWords";
import type { UpdateState } from "@/proto/UpdateState";

function state(over: Partial<UpdateState>): UpdateState {
  return {
    current_version: "0.3.0",
    auto_check: true,
    last_check: null,
    phase: { kind: "idle" },
    available: null,
    skipped_version: null,
    snoozed: false,
    updated_to: null,
    ...over,
  };
}

const failed = (message: string) => ({ kind: "install_failed", message }) as const;

describe("the sentence after a failed install", () => {
  it("says nothing about the version when the update that failed was for that version", () => {
    // The update was to 0.3.0 and 0.3.0 is what runs: "still on 0.3.0" reads as if the update
    // never reached this PC (#60).
    expect(installFailedSentence("Access to the path is denied.", "0.3.0", "0.3.0")).toBe(
      "Access to the path is denied.",
    );
  });

  it("names the version that is installed when it is another one", () => {
    expect(installFailedSentence("The installer stopped early.", "0.2.1", "0.3.0")).toBe(
      "The installer stopped early. canvas-rgb is still on 0.2.1.",
    );
  });

  it("treats a v-prefixed version as the same version", () => {
    expect(installFailedSentence("It did not finish.", "0.3.0", "v0.3.0")).toBe("It did not finish.");
  });

  it("names the version when nothing is known about the update that failed", () => {
    expect(installFailedSentence("It did not finish.", "0.2.1", null)).toBe(
      "It did not finish. canvas-rgb is still on 0.2.1.",
    );
  });

  it("keeps the plain sentence when the message is not one a person can read", () => {
    expect(installFailedSentence("exit code 10", "0.3.0", "0.3.0")).toBe(
      "The installer stopped before it finished.",
    );
  });
});

describe("the state line after a failed install", () => {
  it("does not say the failed version is still the one running", () => {
    const u = state({
      phase: failed("Access to the path 'hidapi-hotplug.dll' is denied."),
      available: { version: "0.3.0", notes: "", pub_date: null },
    });
    expect(stateLine(u)).toBe("Access to the path 'hidapi-hotplug.dll' is denied.");
  });

  it("says which version is installed when the failed update was for another one", () => {
    const u = state({
      current_version: "0.2.1",
      phase: failed("The installer stopped early."),
      available: { version: "0.3.0", notes: "", pub_date: null },
    });
    expect(stateLine(u)).toBe("The installer stopped early. canvas-rgb is still on 0.2.1.");
  });
});
