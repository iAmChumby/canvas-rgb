import type { Metadata, Viewport } from "next";
import "@fontsource-variable/schibsted-grotesk/index.css";
import "@fontsource-variable/jetbrains-mono/index.css";
// The editor's own stylesheet, read-only. The site is the editor's design: the same tokens,
// the same controls, the same hardware layer.
import "@editor/styles/tokens.css";
import "@editor/styles/app.css";
import "@editor/styles/motion.css";
import "@editor/styles/hardware.css";
import "./site.css";

export const metadata: Metadata = {
  title: "canvas-rgb",
  description:
    "RGB software shouldn't take a gigabyte of RAM. canvas-rgb runs your lights, fans, mouse and controller in a fraction of the memory.",
  icons: {
    icon: `${process.env.NEXT_PUBLIC_BASE_PATH ?? ""}/favicon.ico`,
    apple: `${process.env.NEXT_PUBLIC_BASE_PATH ?? ""}/icon.png`,
  },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
};

// Runs before the first paint, so the page never flashes the wrong theme. The choice is per
// viewer and lives in localStorage: "light" or "dark" pins it, nothing stored follows the
// system preference, exactly like the editor's tokens.
// The ?theme= pin is for screenshots of both themes without touching localStorage; with no
// query the stored choice or the system preference decides, as before.
const THEME_SCRIPT = `try{var q=new URLSearchParams(location.search).get("theme");var t=q||localStorage.getItem("canvas-rgb.theme");if(t==="light"){document.documentElement.setAttribute("data-theme","light")}else if(t==="dark"){document.documentElement.setAttribute("data-theme","dark")}}catch(e){}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
        {/* Before the stylesheets arrive, hold the layout viewport at the device width. Overflow
            itself is fixed at its source in the docs CSS; clipping here would hide a real
            overflow and make a width check unable to fail. */}
        <style dangerouslySetInnerHTML={{ __html: "html,body{max-width:100%}pre,table{max-width:100%}" }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
