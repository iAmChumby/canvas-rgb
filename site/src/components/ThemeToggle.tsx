"use client";

import { useEffect, useState } from "react";
import { Moon, Sun } from "@phosphor-icons/react";
import { IconButton } from "@editor/components/Button";

/**
 * The theme button. The page follows the system preference until it is touched; after that the
 * choice is kept in localStorage, the same key the editor uses.
 *
 * The icon is drawn by the stylesheet, not by React: which one shows depends on the theme,
 * and the theme is set on <html> before the first paint. That keeps the button from
 * changing shape when the page hydrates.
 */
export function ThemeToggle() {
  const [theme, setTheme] = useState<"light" | "dark">("dark");

  // Read the theme the page already has, so the button starts on the truth.
  useEffect(() => {
    const root = document.documentElement;
    const set = root.getAttribute("data-theme");
    const resolved =
      set === "light"
        ? "light"
        : set === "dark"
          ? "dark"
          : window.matchMedia("(prefers-color-scheme: light)").matches
            ? "light"
            : "dark";
    setTheme(resolved);
  }, []);

  const toggle = () => {
    const next = theme === "light" ? "dark" : "light";
    setTheme(next);
    document.documentElement.setAttribute("data-theme", next);
    try {
      localStorage.setItem("canvas-rgb.theme", next);
    } catch {
      // storage is off; the theme still holds until the page is reloaded
    }
  };

  return (
    <IconButton
      label="Switch between the light and the dark theme"
      icon={
        <>
          <Sun className="site-theme-sun" size={18} />
          <Moon className="site-theme-moon" size={18} />
        </>
      }
      onClick={toggle}
    />
  );
}
