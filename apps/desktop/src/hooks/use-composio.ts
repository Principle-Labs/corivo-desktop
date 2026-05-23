// React Query hooks for the Composio gateway (Settings UI).
//
// The MCP tool surface is wired in the Rust runner; these hooks only
// drive the connection-management surface (list / start OAuth /
// disconnect). All three commands round-trip through corivo-api so
// no Composio master key crosses the IPC boundary.
//
// Capability-gated by `connectors`: in the open-source build the
// matching Tauri commands aren't registered (their entries in
// `src-tauri/src/lib.rs` invoke_handler! sit behind
// `#[cfg(feature = "corivo-cloud")]`), so a stray IPC call would
// just bounce with `command not found`. The `enabled` flag on the
// query prevents react-query from firing that doomed call in the
// first place; the Settings UI hides the consuming surfaces
// independently as a second line of defense.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import {
  composioCreateConnectionLink,
  composioDisconnect,
  composioListConnections,
} from "@/lib/tauri"
import { useCapabilities } from "@/hooks/use-capabilities"

const QUERY_KEY = ["composio", "connections"] as const

export function useComposioConnections() {
  const { data: caps } = useCapabilities()
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: composioListConnections,
    // Composio's OAuth callback doesn't notify the desktop directly —
    // user finishes consent in the system browser, then comes back to
    // Corivo. Refetch on focus so the new connection appears without
    // a manual refresh.
    refetchOnWindowFocus: true,
    enabled: caps?.connectors ?? false,
  })
}

function useInvalidate() {
  const qc = useQueryClient()
  return () => qc.invalidateQueries({ queryKey: QUERY_KEY })
}

export function useStartComposioConnection() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (toolkitSlug: string) =>
      composioCreateConnectionLink(toolkitSlug),
    // The redirect kicks off async; the connection becomes ACTIVE
    // only after the user finishes consent. Optimistically invalidate
    // so the row appears as INITIATED right away.
    onSuccess: invalidate,
  })
}

export function useDisconnectComposio() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (connectionId: string) => composioDisconnect(connectionId),
    onSuccess: invalidate,
  })
}
