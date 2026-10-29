import { Shot } from "./Shot";

export function AppShot() {
  return (
    <section className="site-section" id="editor">
      <div className="site-section-head">
        <h2 className="site-h2">The editor</h2>
        <p className="site-sub">
          Build your case on screen, then light it. The editor only costs memory while it&apos;s
          open. Close it and its 360 MB goes with it.
        </p>
      </div>
      <Shot
        name="layout"
        alt="The canvas-rgb editor: a desk layout with the devices placed on it and an effect running across them."
      />
    </section>
  );
}
