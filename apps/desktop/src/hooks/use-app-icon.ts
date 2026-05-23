import { useQuery } from "@tanstack/react-query";

import { getAppIcon } from "@/lib/tauri";

export const APP_ICON_QUERY_KEY = ["app-icon"] as const;

/// Resolve a macOS bundle id to a PNG data URL for the foreground app's
/// icon. Returns `undefined` while loading, `null` if the host can't
/// produce one (non-macOS or unresolvable bundle id), or the data URL
/// when ready. Rust side memoizes forever per bundle id; the React
/// Query layer here makes hits across components share one in-flight
/// promise + cache result for the session.
export function useAppIcon(bundleId: string | null | undefined) {
  return useQuery({
    queryKey: [...APP_ICON_QUERY_KEY, bundleId ?? null],
    queryFn: () => (bundleId ? getAppIcon(bundleId) : Promise.resolve(null)),
    enabled: !!bundleId,
    // App icons don't change while the app runs — never refetch.
    staleTime: Infinity,
    gcTime: Infinity,
  });
}
