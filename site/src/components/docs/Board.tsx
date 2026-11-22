/** C's hardware boards, built from the editor's own keys and LEDs. No new colours. */

function Keys({ labels }: { labels: Array<[string, boolean]> }) {
  return (
    <div className="c-board-keys" aria-hidden="true">
      {labels.map(([label, latched]) => (
        <span className="c-key" data-latched={latched ? "true" : undefined} key={label}>
          <span className="led" />
          {label}
        </span>
      ))}
    </div>
  );
}

export function DaemonBoard() {
  return (
    <aside className="c-board" aria-label="Daemon at a glance">
      <div className="c-board-top">
        <span className="c-board-title">Daemon / status</span>
        <span className="c-board-lights">
          <span className="led" data-on="ok" />
          <span className="led" data-on="a3" />
          <span className="led" />
          <span className="led" />
        </span>
      </div>
      <div className="c-board-screen">
        <div className="c-board-row">
          <span>Endpoint</span>
          <b>127.0.0.1:9430</b>
        </div>
        <div className="c-board-row">
          <span>Power</span>
          <b data-tone="ok">running</b>
        </div>
        <div className="c-board-row">
          <span>Service</span>
          <b data-tone="a3">connected</b>
        </div>
      </div>
      <Keys labels={[["Status", true], ["Devices", false], ["Effects", false], ["Fans", false]]} />
    </aside>
  );
}

export function EffectBoard() {
  return (
    <aside className="c-board" aria-label="Effect at a glance">
      <div className="c-board-top">
        <span className="c-board-title">Effect / comet</span>
        <span className="c-board-lights">
          <span className="led" data-on="a2" />
          <span className="led" data-on="a3" />
          <span className="led" />
        </span>
      </div>
      <div className="c-board-screen">
        <div className="c-board-row">
          <span>Canvas</span>
          <b>320 x 200</b>
        </div>
        <div className="c-board-row">
          <span>Frame budget</span>
          <b>100 ms</b>
        </div>
        <div className="c-board-row">
          <span>Load budget</span>
          <b>2 s</b>
        </div>
      </div>
      <Keys labels={[["File", true], ["Props", false], ["Draw", false], ["Tile", false]]} />
    </aside>
  );
}
