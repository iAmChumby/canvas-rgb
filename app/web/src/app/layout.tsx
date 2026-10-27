import type { Metadata } from "next";
import "@fontsource-variable/schibsted-grotesk/index.css";
import "@fontsource-variable/jetbrains-mono/index.css";
import "@/styles/tokens.css";
import "@/styles/app.css";
import "@/styles/motion.css";
import { Shell } from "@/shell/Shell";

export const metadata: Metadata = {
  title: "canvas-rgb",
  description: "Lighting and fan controller: layout, effects, devices, fans and settings.",
};

// Runs before first paint. The theme choice is per viewer (localStorage); "system" removes the
// attribute so the prefers-color-scheme set applies. Anything else keeps the dark default.
// The motion preference works the same way: "reduced" turns the editor's motion off.
const THEME_SCRIPT = `try{var t=localStorage.getItem("canvas-rgb.theme");if(t==="light"){document.documentElement.setAttribute("data-theme","light")}else if(t==="system"){document.documentElement.removeAttribute("data-theme")}}catch(e){}try{var m=localStorage.getItem("canvas-rgb.motion");if(m==="reduced"){document.documentElement.setAttribute("data-motion","reduced")}}catch(e){}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" data-theme="dark" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  );
}
