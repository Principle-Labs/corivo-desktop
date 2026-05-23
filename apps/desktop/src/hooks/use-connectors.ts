// React Query hooks for the connector framework.
//
// Generic over connector id — Notion / Slack / etc. land later without
// new hooks. Connect is intentionally long-running (waits for the
// system browser; up to 5 min); UI should disable controls during the
// mutation and show a spinner.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import {
  connectorConnect,
  connectorDisable,
  connectorDisconnect,
  connectorEnable,
  connectorInstallMcp,
  connectorProviderConnect,
  connectorsList,
} from "@/lib/tauri"

const QUERY_KEY = ["connectors"] as const

export function useConnectors() {
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: connectorsList,
    refetchOnWindowFocus: false,
  })
}

function useInvalidate() {
  const qc = useQueryClient()
  return () => qc.invalidateQueries({ queryKey: QUERY_KEY })
}

export function useEnableConnector() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (id: string) => connectorEnable(id),
    onSuccess: invalidate,
  })
}

export function useDisableConnector() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (id: string) => connectorDisable(id),
    onSuccess: invalidate,
  })
}

export function useConnectConnector() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (id: string) => connectorConnect(id),
    onSuccess: invalidate,
  })
}

/** Bulk connect: opens ONE OAuth flow for the union of every listed
 *  connector's manifest scopes, then binds + enables all of them on the
 *  resulting account. Drives the `ProviderCard` "连接 Google (N 项)"
 *  button. Long-running like {@link useConnectConnector} — UI should
 *  show a spinner until the mutation resolves. */
export function useConnectProvider() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: ({
      provider,
      connectorIds,
    }: {
      provider: string
      connectorIds: string[]
    }) => connectorProviderConnect(provider, connectorIds),
    onSuccess: invalidate,
  })
}

/** Install + authorize an `mcpServer`-shape connector (Linear, etc).
 *  Long-running like {@link useConnectConnector} — UI should disable
 *  controls and show a spinner until the mutation resolves. */
export function useInstallMcpConnector() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (id: string) => connectorInstallMcp(id),
    onSuccess: invalidate,
  })
}

export function useDisconnectConnector() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: (id: string) => connectorDisconnect(id),
    onSuccess: invalidate,
  })
}
