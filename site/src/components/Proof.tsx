import { Figure } from "@editor/components/Readout";

const BENCHMARK = "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/benchmark.md";

/** The three numbers, from docs/benchmark.md. Nothing else is measured. */
const READOUTS = [
  { label: "RAM", mine: "60", mineUnit: "MB", theirs: "819", theirsUnit: "MB" },
  { label: "CPU", mine: "0.6", mineUnit: "%", theirs: "2.6", theirsUnit: "%" },
  { label: "Disk", mine: "80", mineUnit: "MB", theirs: "2.4", theirsUnit: "GB" },
];

export function Proof() {
  return (
    <section className="site-section site-proof" id="proof">
      <div className="site-proof-head">
        <h2 className="site-h2">Measured</h2>
        <p className="site-note">One PC, 60 seconds per app.</p>
      </div>
      <div className="site-readout">
        {READOUTS.map((r) => (
          <div className="site-readout-cell" key={r.label}>
            <span className="site-readout-label">{r.label}</span>
            <span className="site-readout-values">
              <span className="site-readout-mine">
                <span className="site-readout-led" data-tone="mine" aria-hidden="true" />
                <Figure unit={r.mineUnit}>{r.mine}</Figure>
              </span>
              <span className="site-readout-vs">vs</span>
              <span className="site-readout-theirs">
                <Figure unit={r.theirsUnit}>{r.theirs}</Figure>
              </span>
            </span>
          </div>
        ))}
      </div>
      <p className="site-note">
        SignalRGB 2.5.80, ZEUS CAST 2.4.2.2 and the Attack Shark X6 app, one at a time.{" "}
        <a className="site-readout-link" href={BENCHMARK}>
          docs/benchmark.md
        </a>
      </p>
    </section>
  );
}
