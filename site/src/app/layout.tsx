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
const THEME_SCRIPT = `try{var t=localStorage.getItem("canvas-rgb.theme");if(t==="light"){document.documentElement.setAttribute("data-theme","light")}else if(t==="dark"){document.documentElement.setAttribute("data-theme","dark")}}catch(e){}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
