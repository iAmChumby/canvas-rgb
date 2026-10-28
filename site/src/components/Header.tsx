import { GithubLogo } from "@phosphor-icons/react/ssr";
import { Wordmark } from "@editor/components/BrandMark";
import { Tooltip } from "@editor/components/Tooltip";
import { ThemeToggle } from "./ThemeToggle";

const REPO = "https://github.com/iAmChumby/canvas-rgb";

const LINKS = [
  { href: "#features", label: "Features" },
  { href: "#editor", label: "Editor" },
  { href: "#install", label: "Install" },
  { href: "#thanks", label: "Thanks" },
];

export function Header() {
  return (
    <header className="site-header">
      <div className="site-header-inner">
        <a className="site-brand" href="#top" aria-label="canvas-rgb">
          <Wordmark />
        </a>
        <nav className="site-nav" aria-label="Sections">
          {LINKS.map((l) => (
            <a key={l.href} className="site-nav-link" href={l.href}>
              {l.label}
            </a>
          ))}
        </nav>
        <div className="site-header-actions">
          <Tooltip label="canvas-rgb on GitHub">
            <a className="ibtn site-github" href={REPO} aria-label="canvas-rgb on GitHub">
              <GithubLogo size={18} />
            </a>
          </Tooltip>
          <ThemeToggle />
        </div>
      </div>
    </header>
  );
}
