/** The four things canvas-rgb does, in Luke's words. Facts from README.md only. */
const FEATURES = [
  {
    title: "Lighting",
    text: "Every device sits on one layout, and your SignalRGB effects run on it unchanged.",
  },
  {
    title: "Fans",
    text: "Fan curves per header, and every fan goes back to BIOS when the service stops.",
  },
  {
    title: "Mouse and controller",
    text: "The Attack Shark X6 and the 8BitDo Ultimate 2 are set up from inside canvas-rgb, so their vendor apps can stay closed.",
  },
  {
    title: "For tinkerers",
    text: "A local API, a command line, and new effects are one HTML file each.",
  },
];

export function Features() {
  return (
    <section className="site-section" id="features">
      <div className="site-section-head">
        <h2 className="site-h2">What it does</h2>
      </div>
      <div className="site-features">
        {FEATURES.map((f) => (
          <div className="site-feature" key={f.title}>
            <h3 className="site-feature-title">{f.title}</h3>
            <p className="site-feature-text">{f.text}</p>
          </div>
        ))}
      </div>
    </section>
  );
}
