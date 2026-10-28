import { ButtonLink } from "@editor/components/Button";
import { CopyButton } from "./CopyButton";

const RELEASES = "https://github.com/iAmChumby/canvas-rgb/releases/latest";
const PAWNIO = "https://github.com/namazso/PawnIO";
const SETUP = "canvas-rgb-setup-0.1.0.exe";

const COMMANDS = `powershell -ExecutionPolicy Bypass -File installer\\setup.ps1 -WhatIf
powershell -ExecutionPolicy Bypass -File installer\\setup.ps1`;

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
      <p className="site-install-warning">
        Windows will warn about an unknown publisher on first run; the installer is not
        code-signed yet.
      </p>
      <h3 className="site-install-sub">Build from source</h3>
      <div className="site-install">
        <div className="site-code">
          <pre>
            <code>{COMMANDS}</code>
          </pre>
          <div className="site-code-actions">
            <CopyButton text={COMMANDS} />
          </div>
        </div>
        <div className="site-install-notes">
          <p className="site-note">
            One admin prompt. It checks OpenRGB&apos;s SHA-256 before it runs anything.
          </p>
          <p className="site-note">
            Fan control needs the{" "}
            <a className="site-readout-link" href={PAWNIO}>
              PawnIO
            </a>{" "}
            driver, which the installer does not ship.
          </p>
        </div>
      </div>
    </section>
  );
}
