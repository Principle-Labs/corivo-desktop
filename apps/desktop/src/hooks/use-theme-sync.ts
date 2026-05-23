import { useEffect } from "react";
import { useConfig } from "@/hooks/use-config";
import type { ThemePreference } from "@/lib/types";

/**
 * Sync the user's `Config.app.theme` preference to the document root
 * via `[data-theme="light"|"dark"]`. CSS variables in
 * `packages/ui/src/styles/globals.css` then resolve to the right
 * palette without any component code change.
 *
 * Behavior:
 *   - `light` / `dark` — locked
 *   - `system` (default) — follows macOS appearance via the
 *     `(prefers-color-scheme: dark)` media query, and re-applies on
 *     change while the app is open
 *
 * Mounted once at the app root (see `app-boot.tsx` ThemeProvider).
 * Quick Ask + notification overlay each have their own React root and
 * read the Config independently.
 */
export function useThemeSync() {
  const { config } = useConfig();
  const preference: ThemePreference = config?.app.theme ?? "system";

  useEffect(() => {
    const root = document.documentElement;
    const mql = window.matchMedia("(prefers-color-scheme: dark)");

    const apply = () => {
      const resolved =
        preference === "system" ? (mql.matches ? "dark" : "light") : preference;
      root.dataset.theme = resolved;
    };

    apply();

    if (preference === "system") {
      mql.addEventListener("change", apply);
      return () => mql.removeEventListener("change", apply);
    }
    // `light` / `dark` — locked, no listener needed.
    return undefined;
  }, [preference]);
}
