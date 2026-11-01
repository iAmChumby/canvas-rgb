import { ButtonLink } from "@editor/components/Button";
import { CopyButton } from "./CopyButton";

const RELEASES = "https://github.com/iAmChumby/canvas-rgb/releases/latest";
const PAWNIO = "https://github.com/namazso/PawnIO";
const REPO = "https://github.com/iAmChumby/canvas-rgb";
const SETUP = "canvas-rgb-setup-0.5.0.exe";

// The installer's SHA-256, read from the release's own .sha256 asset when the site is built
// (site/scripts/setup-sha.mjs). It is not written here, so the page cannot show a hash the
// release does not carry.
const SHA256 = process.env.NEXT_PUBLIC_SETUP_SHA256;
if (!SHA256) {
  throw new Error("the installer's SHA-256 is missing: the site was built without setup-sha.");
}

// The check a visitor runs on the file they downloaded. The Copy button copies this line and
// nothing else: the hash is not a command, and pasting it after the command is an error in
// PowerShell.
const CHECK = `Get-FileHash .\\${SETUP}`;

// Building from source. A .cmd wrapper, so no PowerShell command line is shown or copied: that
// is the shape of a ClickFix attack, and ad blockers warn about it (#79).
const SOURCE = `git clone ${REPO}
cd canvas-rgb
installer\\setup.cmd -WhatIf
installer\\setup.cmd`;

export function Install() {
  return (
    <section className="site-section" id="install">
      <div className="site-section-head">
        <h2 className="site-h2">Install</h2>
      </div>
      <div className="site-install-download">
        <ButtonLink href={RELEASES} variant="primary">
          Download
        </ButtonLink>
        <span className="site-install-file fig">{SETUP}</span>
      </div>

      <h3 className="site-install-sub">Check the file</h3>
      <div className="site-install">
        <div className="site-code">
          <pre>
            <code>{CHECK}</code>
          </pre>
          <div className="site-code-actions">
            <CopyButton text={CHECK} />
          </div>
          <p className="site-code-label">Expected SHA-256</p>
          <pre>
            <code>{SHA256}</code>
          </pre>
        </div>
        <div className="site-install-notes">
          <p className="site-note">
            The installer isn&apos;t signed yet, so Windows flags an unknown publisher the first
            time. This hash is how you know the file is ours: run the command, and all 64
            characters have to match, whatever the case: Get-FileHash prints the hash in
            capitals.
          </p>
        </div>
      </div>

      <h3 className="site-install-sub">Build from source</h3>
      <div className="site-install">
        <div className="site-code">
          <pre>
            <code>{SOURCE}</code>
          </pre>
          <div className="site-code-actions">
            <CopyButton text={SOURCE} />
          </div>
        </div>
        <div className="site-install-notes">
          <p className="site-note">
            One admin prompt. OpenRGB gets downloaded and its SHA-256 checked before anything
            runs.
          </p>
          <p className="site-note">
            Fan control needs the{" "}
            <a className="site-readout-link" href={PAWNIO}>
              PawnIO
            </a>{" "}
            driver. Grab it separately.
          </p>
        </div>
      </div>
    </section>
  );
}
