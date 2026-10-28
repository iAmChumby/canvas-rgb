import type { Metadata } from "next";
import "@fontsource-variable/schibsted-grotesk/index.css";
import "@fontsource-variable/jetbrains-mono/index.css";
import "@/styles/tokens.css";
import "@/styles/app.css";
import "@/styles/motion.css";
import "@/styles/hardware.css";
import { Shell } from "@/shell/Shell";

export const metadata: Metadata = {
  title: "canvas-rgb",
  description: "Lighting and fan controller: layout, effects, devices, fans and settings.",
};

// Runs before first paint. The theme follows Windows by default: with no data-theme attribute the
// prefers-color-scheme rules in tokens.css apply, in the stylesheet itself, so the first paint is
// already right. Only an explicit "light" or "dark" choice (localStorage, per viewer) sets the
// attribute. The motion preference works the same way: "reduced" turns the editor's motion off.
const THEME_SCRIPT = `try{var t=localStorage.getItem("canvas-rgb.theme");if(t==="light"||t==="dark"){document.documentElement.setAttribute("data-theme",t)}}catch(e){}try{var m=localStorage.getItem("canvas-rgb.motion");if(m==="reduced"){document.documentElement.setAttribute("data-motion","reduced")}}catch(e){}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  );
}
