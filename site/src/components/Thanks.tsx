/** The projects canvas-rgb builds on, from docs/thanks.md. */
const THANKS = [
  {
    name: "OpenRGB",
    href: "https://codeberg.org/OpenRGB/OpenRGB",
    note: "Talking to the hardware: ARGB controllers, RAM over SMBus, a different chip and protocol for every vendor.",
    more: null,
  },
  {
    name: "Hypercolor",
    href: "https://github.com/hyperb1iss/hypercolor",
    note: "A Rust lighting engine: how to speak OpenRGB's protocol from Rust, and how to reach the fan chip.",
    more: null,
  },
  {
    name: "PawnIO",
    href: "https://github.com/namazso/PawnIO",
    note: "A signed driver that lets user code reach the Super I/O chip safely. The fan control runs on it.",
    more: null,
  },
  {
    name: "nct6687d",
    href: "https://github.com/Fred78290/nct6687d",
    note: "The register map of the fan chip came from their drivers.",
    more: [
      { label: "LibreHardwareMonitor", href: "https://github.com/LibreHardwareMonitor/LibreHardwareMonitor" },
    ],
  },
  {
    name: "ATKsoft",
    href: "https://github.com/grivia0/ATKsoft",
    note: "The Attack Shark X6 read-back handshake and its report layouts.",
    more: [
      { label: "prodmutant", href: "https://github.com/prodmutant/Attack-Shark-X3" },
      { label: "AlternativeLua", href: "https://github.com/AlternativeLua/attack-shark-x3-driver" },
      { label: "DavBfr", href: "https://github.com/DavBfr/shark-x3" },
      { label: "libratbag", href: "https://github.com/libratbag/libratbag" },
    ],
  },
  {
    name: "8bitdo-Linux-Software",
    href: "https://github.com/ascendedent/8bitdo-Linux-Software",
    note: "The only public map of the 8BitDo Ultimate 2's configuration channel.",
    more: null,
  },
  {
    name: "SDL",
    href: "https://github.com/libsdl-org/SDL",
    note: "Its 8BitDo driver documents the controller's input reports.",
    more: null,
  },
  {
    name: "SignalRGB",
    href: "https://signalrgb.com",
    note: "The HTML effect format, so the effects people already wrote for it run here.",
    more: null,
  },
];

export function Thanks() {
  return (
    <section className="site-section" id="thanks">
      <div className="site-section-head">
        <h2 className="site-h2">Thanks</h2>
        <p className="site-sub">
          canvas-rgb stands on other people&apos;s work. What each one made possible is in{" "}
          <a
            className="site-readout-link"
            href="https://github.com/iAmChumby/canvas-rgb/blob/main/docs/thanks.md"
          >
            docs/thanks.md
          </a>
          .
        </p>
      </div>
      <ul className="site-thanks">
        {THANKS.map((t) => (
          <li className="site-thanks-item" key={t.name}>
            <span className="site-thanks-name">
              <a href={t.href}>{t.name}</a>
            </span>
            <span className="site-thanks-note">{t.note}</span>
            {t.more && (
              <span className="site-thanks-more">
                Also:{" "}
                {t.more.map((m, i) => (
                  <span key={m.href}>
                    {i > 0 && ", "}
                    <a href={m.href}>{m.label}</a>
                  </span>
                ))}
              </span>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
