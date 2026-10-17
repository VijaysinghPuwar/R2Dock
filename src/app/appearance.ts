import { useEffect } from "react";
import type { Preferences } from "../ipc/bindings/Preferences";
import type { Theme } from "../ipc/bindings/Theme";

/** "dark" or "paper" for the given theme preference. */
export function resolveTheme(theme: Theme, prefersLight: boolean): "dark" | "paper" {
  if (theme === "system") return prefersLight ? "paper" : "dark";
  return theme;
}

function prefersLight(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: light)").matches;
}

/** Write theme, density, and text size to the root element. */
export function applyAppearance(prefs: Pick<Preferences, "theme" | "density" | "textSize">): void {
  const root = document.documentElement;
  root.dataset.theme = resolveTheme(prefs.theme, prefersLight());
  root.dataset.themePreference = prefs.theme;
  root.dataset.density = prefs.density;
  root.dataset.textSize = prefs.textSize;
}

/** Keep the root element in sync with preferences and the OS color scheme. */
export function useAppearance(prefs: Preferences): void {
  const { theme, density, textSize } = prefs;
  useEffect(() => {
    applyAppearance({ theme, density, textSize });
    if (theme !== "system" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => applyAppearance({ theme, density, textSize });
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme, density, textSize]);
}
