export type Theme = "system" | "light" | "dark";
const THEME_KEY = "suite:theme";

export function loadTheme(): Theme {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // Storage blocked: follow the system.
  }
  return "system";
}

export function saveTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Not remembered.
  }
}

/** Puts the choice on <html>, where suite.css reads it. "System" leaves the attribute off so
 * that prefers-color-scheme decides. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

export function isDark(theme: Theme): boolean {
  if (theme !== "system") return theme === "dark";
  return !window.matchMedia?.("(prefers-color-scheme: light)").matches;
}

// Before the first paint, so a saved choice does not flash the other theme.
applyTheme(loadTheme());
