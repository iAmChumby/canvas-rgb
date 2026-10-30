import { ButtonLink } from "@editor/components/Button";
import { HeroWordmark } from "./HeroWordmark";

const REPO = "https://github.com/iAmChumby/canvas-rgb";
const RELEASES = "https://github.com/iAmChumby/canvas-rgb/releases/latest";

export function Hero() {
  return (
    <section className="site-hero" id="top">
      <div className="site-hero-inner">
        <HeroWordmark />
        <h1 className="site-h1">RGB software shouldn&apos;t take a gigabyte of RAM.</h1>
        <p className="site-lead">
          That changes now. canvas-rgb runs your lights, fans, mouse and controller in a fraction
          of the memory.
        </p>
        <div className="site-hero-actions">
          <ButtonLink href={RELEASES} variant="primary">
            Download
          </ButtonLink>
          <ButtonLink href={REPO}>Source</ButtonLink>
        </div>
        <p className="site-hero-note">Windows 11 · v0.1.2 · Apache-2.0</p>
      </div>
    </section>
  );
}
