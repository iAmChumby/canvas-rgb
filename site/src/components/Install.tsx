import { ButtonLink } from "@editor/components/Button";
import { CopyButton } from "./CopyButton";

const RELEASES = "https://github.com/iAmChumby/canvas-rgb/releases/latest";
const PAWNIO = "https://github.com/namazso/PawnIO";
const SETUP = "canvas-rgb-setup-0.1.3.exe";

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
        Windows flags an unknown publisher the first time. The installer isn&apos;t signed yet.
      </p>
      <h3 className="site-install-sub">Developer build</h3>
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
