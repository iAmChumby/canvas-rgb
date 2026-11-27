/** The four things canvas-rgb does, in Luke's words. Facts from README.md only. */
const FEATURES = [
  {
    title: "Lighting",
    text: "Your Signal effects run here untouched. Same HTML files, drawn in Rust with no browser underneath. Put every device on one layout and an effect runs across the lot.",
  },
  {
    title: "Fans",
    text: "Curves per header, with BIOS taking the fans back the moment the service dies or a sensor goes quiet.",
  },
  {
    title: "Mouse and controller",
    text: "The Attack Shark X6 and 8BitDo Ultimate 2 apps can stay closed for good. Their settings live in Canvas now.",
  },
  {
    title: "For developers",
    text: "There's a local API on port 9430. The CLI talks to it too, and --json makes any of it scriptable. New effects are one HTML file.",
  },
];

export function Features() {
  return (
    <section className="site-section" id="features">
      <div className="site-section-head">
        <h2 className="site-h2">Same job. Then some.</h2>
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
