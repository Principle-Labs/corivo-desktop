import { useMemo } from "react"
import { Loader2 } from "lucide-react"
import { useRouter } from "@tanstack/react-router"

import { Button } from "@repo/ui/components/button"

import { useTranslation } from "@/i18n"
import { useCapabilities } from "@/hooks/use-capabilities"
import {
  useComposioConnections,
  useDisconnectComposio,
  useStartComposioConnection,
} from "@/hooks/use-composio"
import { useConnectors } from "@/hooks/use-connectors"
import { useActiveThreadStore } from "@/stores/active-thread-store"
import { closeSettingsDialog } from "@/stores/settings-dialog-store"
import type {
  ComposioConnection,
  ConnectorSummary,
  LocalizedString,
} from "@corivo/shared-types"

import { FieldGroup, SectionHeader, SettingsSkeleton } from "./settings-shared"

// All action buttons on the right of an integration row share the same width
// so the column reads as a clean vertical edge instead of a ragged one.
const ACTION_BUTTON_CLASS = "min-w-24"

/**
 * Settings → Integrations.
 *
 * Two groups now:
 *
 * - **Accounts** — SaaS integrations (Gmail / Slack / Notion / ...).
 *   Backed by the Corivo backend's Composio gateway in apps/api; the
 *   master Composio API key never crosses to the desktop. OAuth happens
 *   in the system browser (Composio hosts the consent screen).
 * - **Command-line tools** — local CLI binaries (Lark CLI, GitHub CLI).
 *   No OAuth on this device; clicking 安装 opens an /ask thread where
 *   the agent walks the user through `brew install ...` + `gh auth login`.
 *
 * The previous OAuth-provider and vendor-MCP groups (Google one-shot card,
 * Linear / GitLab MCP rows) have been removed — those SaaS now flow
 * through the Accounts group via Composio.
 */
export function IntegrationsSection() {
  const { lang } = useTranslation()
  const { data: capabilities } = useCapabilities()
  const { data: connectors, isLoading, isError, error } = useConnectors()
  const isZh = lang === "zh"

  // Capability gate. Settings-dialog already hides this tab when
  // `connectors` is off (see `components/settings/settings-dialog.tsx`);
  // this early-return is the belt-and-suspenders guarantee for any
  // future caller that mounts the section directly.
  if (!capabilities?.connectors) {
    return null
  }

  if (isLoading) {
    return <SettingsSkeleton />
  }

  if (isError || !connectors) {
    return (
      <div className="max-w-2xl space-y-6">
        <SectionHeader
          title={isZh ? "集成" : "Integrations"}
          description={
            isZh
              ? "连接 Corivo 跟你常用的服务。"
              : "Connect Corivo to the services you use."
          }
        />
        <div className="rounded-md border border-destructive/40 bg-destructive/5 p-4 text-xs text-destructive">
          {isZh
            ? "无法加载集成列表。请确认 Tauri 后端已重新构建（在 monorepo 根目录运行 pnpm app:dev 重启），然后刷新该页面。"
            : "Could not load integrations. Make sure the Tauri backend has been rebuilt (restart pnpm app:dev from the monorepo root) and reload this page."}
          {error ? (
            <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-all font-mono text-[10px] text-destructive/80">
              {String(error instanceof Error ? error.message : error)}
            </pre>
          ) : null}
        </div>
      </div>
    )
  }

  // Post-Composio cutover the only local connector framework consumer
  // left is `cliInstall`. The Composio-backed SaaS list is rendered by
  // <ComposioGroup /> against its own backend hooks.
  const cliInstallRows = connectors.filter(
    (c) => c.manifest.auth.type === "cliInstall",
  )

  return (
    <div className="max-w-2xl space-y-6">
      <SectionHeader
        title={isZh ? "集成" : "Integrations"}
        description={
          isZh
            ? "连接 Corivo 跟你常用的服务。"
            : "Connect Corivo to the services you use."
        }
      />

      <FieldGroup title={isZh ? "账号" : "Accounts"}>
        <ComposioGroup lang={lang} />
      </FieldGroup>

      {cliInstallRows.length > 0 ? (
        <FieldGroup title={isZh ? "命令行工具" : "Command-line tools"}>
          <ul className="divide-y divide-border/40 rounded-md border border-border/40">
            {cliInstallRows.map((c) => (
              <li key={c.id}>
                <CliInstallRow connector={c} lang={lang} />
              </li>
            ))}
          </ul>
        </FieldGroup>
      ) : null}
    </div>
  )
}

// ──────────────────────────────────────────────────────────────────────
// Composio group — server-proxied SaaS catalog
//
// The MCP tool surface itself is auto-wired in the Rust runner (see
// `enabled_mcp_specs`). This UI is exclusively for managing OAuth
// connections (the "click to connect Gmail" surface).
//
// Toolkit catalog is intentionally hard-coded here: server-side we
// only have authConfigs for the toolkits the operator has enabled in
// the Composio dashboard; if a slug below has no matching authConfig,
// the connect button returns a 404 and we surface that as an error.
// Future: drive this list from the cloud connector service so the
// operator can change toolkits without a desktop rebuild.
// ──────────────────────────────────────────────────────────────────────

// GitHub deliberately omitted — Corivo ships `github-cli` in the
// command-line tools group, and we don't want two GitHub entry points
// in Settings. Keep this list in sync with `DEFAULT_TOOLKITS` in
// `apps/api/src/routes/composio.ts`.
const COMPOSIO_TOOLKITS: ReadonlyArray<{
  slug: string
  name: string
  description: { zh: string; en: string }
}> = [
  {
    slug: "gmail",
    name: "Gmail",
    description: {
      zh: "读取邮件、查找联系人、起草草稿。",
      en: "Read messages, find contacts, draft replies.",
    },
  },
  {
    slug: "slack",
    name: "Slack",
    description: {
      zh: "查找消息、发到指定频道、查询成员。",
      en: "Search messages, post to channels, list members.",
    },
  },
  {
    slug: "notion",
    name: "Notion",
    description: {
      zh: "查找页面、读取内容、创建笔记。",
      en: "Search pages, read content, create notes.",
    },
  },
]

function ComposioGroup({ lang }: { lang: "zh" | "en" }) {
  const isZh = lang === "zh"
  const { data, isLoading, isError, error } = useComposioConnections()

  // Group connections by toolkit slug so a card knows its own status
  // without re-iterating the whole list. Composio allows multiple
  // connections per toolkit, but the UI shows the most-recent one.
  const connByToolkit = useMemo(() => {
    const map = new Map<string, ComposioConnection>()
    for (const conn of data?.connections ?? []) {
      const prev = map.get(conn.toolkitSlug)
      // Prefer ACTIVE over INITIATED/FAILED so the "is connected?"
      // status matches what the user would say casually.
      if (!prev || (prev.status !== "ACTIVE" && conn.status === "ACTIVE")) {
        map.set(conn.toolkitSlug, conn)
      }
    }
    return map
  }, [data])

  if (isLoading) {
    return (
      <div className="rounded-md border border-border/40 p-3 text-xs text-muted-foreground">
        {isZh ? "加载中…" : "Loading…"}
      </div>
    )
  }

  if (isError) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs text-destructive">
        {isZh ? "无法读取云端连接：" : "Could not load cloud connections: "}
        {String(error instanceof Error ? error.message : error)}
      </div>
    )
  }

  return (
    <ul className="divide-y divide-border/40 rounded-md border border-border/40">
      {COMPOSIO_TOOLKITS.map((tk) => (
        <li key={tk.slug}>
          <ComposioToolkitRow
            toolkit={tk}
            connection={connByToolkit.get(tk.slug) ?? null}
            lang={lang}
          />
        </li>
      ))}
    </ul>
  )
}

function ComposioToolkitRow({
  toolkit,
  connection,
  lang,
}: {
  toolkit: (typeof COMPOSIO_TOOLKITS)[number]
  connection: ComposioConnection | null
  lang: "zh" | "en"
}) {
  const isZh = lang === "zh"
  const start = useStartComposioConnection()
  const disconnect = useDisconnectComposio()

  const status = connection?.status ?? null
  const busy = start.isPending || disconnect.isPending

  // Status pill: ACTIVE → green, INITIATED → amber (浏览器还没回来),
  // FAILED/EXPIRED → red. Anything else (rare) → muted.
  const statusInfo = (() => {
    switch (status) {
      case "ACTIVE":
        return {
          label: isZh ? "已连接" : "Connected",
          className: "text-emerald-500",
        }
      case "INITIATED":
        return {
          label: isZh ? "授权中" : "Authorizing",
          className: "text-amber-500",
        }
      case "FAILED":
      case "EXPIRED":
        return {
          label: isZh ? "需要重新连接" : "Reconnect needed",
          className: "text-destructive",
        }
      case null:
        return {
          label: isZh ? "未连接" : "Not connected",
          className: "text-muted-foreground",
        }
      default:
        return { label: String(status), className: "text-muted-foreground" }
    }
  })()

  const handleConnect = () => start.mutate(toolkit.slug)
  const handleDisconnect = () => {
    if (connection) disconnect.mutate(connection.connectionId)
  }

  const isConnected = status === "ACTIVE"

  return (
    <div className="flex items-start gap-3 p-3">
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium tracking-[-0.005em]">
            {toolkit.name}
          </span>
          <span className={`text-[11px] ${statusInfo.className}`}>
            {statusInfo.label}
          </span>
        </div>
        <div className="line-clamp-2 text-xs text-muted-foreground">
          {isZh ? toolkit.description.zh : toolkit.description.en}
        </div>
        {connection?.label ? (
          <div className="truncate text-[11px] text-muted-foreground">
            {connection.label}
          </div>
        ) : null}
        {start.isError ? (
          <div className="text-[11px] text-destructive">
            {String(start.error instanceof Error ? start.error.message : start.error)}
          </div>
        ) : null}
      </div>
      <div className="flex shrink-0 flex-col gap-1.5">
        {isConnected ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className={ACTION_BUTTON_CLASS}
            disabled={busy}
            onClick={handleDisconnect}
          >
            {disconnect.isPending ? (
              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
            ) : null}
            {isZh ? "断开" : "Disconnect"}
          </Button>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className={ACTION_BUTTON_CLASS}
            disabled={busy}
            onClick={handleConnect}
          >
            {start.isPending ? (
              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
            ) : null}
            {status === "INITIATED"
              ? isZh
                ? "继续授权"
                : "Resume"
              : status === "FAILED" || status === "EXPIRED"
                ? isZh
                  ? "重新连接"
                  : "Reconnect"
                : isZh
                  ? "连接"
                  : "Connect"}
          </Button>
        )}
      </div>
    </div>
  )
}

function pickLocalized(s: LocalizedString, lang: "zh" | "en"): string {
  return lang === "zh" ? s.zh : s.en
}

// ──────────────────────────────────────────────────────────────────────
// CliInstallRow — local CLI tools (Lark CLI, GitHub CLI). Clicking 安装
// opens an /ask thread; the agent walks the user through brew / gh auth
// login. 已安装 = `binary_on_path` probe found the binary on PATH.
//
// `ProviderIcon` is co-located here because CliInstallRow uses it for
// the leading avatar; before the Composio cutover several other row
// components shared it too.
// ──────────────────────────────────────────────────────────────────────

function ProviderIcon({
  name,
  avatarUrl,
}: {
  name: string
  avatarUrl: string | null
}) {
  if (avatarUrl) {
    return (
      <img
        src={avatarUrl}
        alt=""
        className="h-9 w-9 shrink-0 rounded-md object-cover"
        referrerPolicy="no-referrer"
      />
    )
  }
  const initial = name.trim().charAt(0).toUpperCase() || "?"
  return (
    <div
      className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-muted text-[14px] font-semibold text-foreground"
      aria-hidden
    >
      {initial}
    </div>
  )
}


function CliInstallRow({
  connector,
  lang,
}: {
  connector: ConnectorSummary
  lang: "zh" | "en"
}) {
  const isZh = lang === "zh"
  const name = pickLocalized(connector.manifest.name, lang)
  const description = pickLocalized(connector.manifest.description, lang)
  const router = useRouter()
  const openNew = useActiveThreadStore((s) => s.openNew)
  const setPendingAutoSend = useActiveThreadStore(
    (s) => s.setPendingAutoSend,
  )

  // Narrowed by the auth.type === "cliInstall" branch in IntegrationsSection.
  const auth =
    connector.manifest.auth.type === "cliInstall"
      ? connector.manifest.auth
      : null
  const installed = connector.cliInstalled === true

  // CO-68: park the prompt as a one-shot auto-send and let AskPage fire
  // it on mount instead of prefilling the composer and waiting for the
  // user to hit Enter. The install instructions are self-contained — by
  // the time the user reaches `/ask`, the agent is already detecting
  // their environment.
  const handleInstall = () => {
    if (!auth) return
    const prompt = pickLocalized(auth.installPrompt, lang)
    openNew()
    setPendingAutoSend(prompt)
    closeSettingsDialog()
    void router.navigate({ to: "/ask" })
  }

  return (
    <div className="flex items-start gap-3 p-3">
      <ProviderIcon name={name} avatarUrl={null} />
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium tracking-[-0.005em]">
            {name}
          </span>
          <span
            className={`text-[11px] ${
              installed ? "text-emerald-500" : "text-muted-foreground"
            }`}
          >
            {installed
              ? isZh
                ? "已安装"
                : "Installed"
              : isZh
                ? "未安装"
                : "Not installed"}
          </span>
        </div>
        <div className="line-clamp-2 text-xs text-muted-foreground">
          {description}
        </div>
        {installed && auth ? (
          <div className="text-[11px] text-muted-foreground">
            <span className="font-mono">{auth.binary}</span>
            {isZh ? " 已在 PATH 中" : " is on PATH"}
          </div>
        ) : null}
      </div>
      <div className="flex shrink-0 flex-col gap-1.5">
        {installed ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className={ACTION_BUTTON_CLASS}
            disabled
            aria-disabled
          >
            {isZh ? "已安装" : "Installed"}
          </Button>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className={ACTION_BUTTON_CLASS}
            onClick={handleInstall}
          >
            {isZh ? "安装" : "Install"}
          </Button>
        )}
      </div>
    </div>
  )
}
