import { useMemo, useState } from "react"
import { useMutation } from "@tanstack/react-query"
import {
  ChevronDown,
  CreditCard,
  LogOut,
  Settings,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@repo/ui/components/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@repo/ui/components/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@repo/ui/components/dropdown-menu"

import { useBillingMe } from "@/hooks/use-billing"
import { useCapabilities } from "@/hooks/use-capabilities"
import { useTranslation } from "@/i18n"
import { authLogout } from "@/lib/tauri"
import { openBillingDialog } from "@/stores/billing-dialog-store"
import { openSettingsDialog } from "@/stores/settings-dialog-store"
import { applyAuthStatus, useUserProfile } from "@/stores/user-profile-store"

/**
 * Sidebar bottom user card — the account anchor of the bottom cluster.
 *
 * Layout:
 *   ┌──────────────────────────────────────┐
 *   │  👤  Sean Liu                    ⌄   │  ← row 1: dropdown trigger
 *   │      $899.99 USD             充值    │  ← row 2: balance + top-up
 *   └──────────────────────────────────────┘
 *
 * The balance row used to live as a separate `BalancePill` between the
 * capture-status row and this card. That made balance read at the same
 * level as a primary capture toggle, which it isn't — balance is an
 * attribute of the account, so it lives inside the account block.
 *
 * Two interactive zones with independent hover highlight + a hairline
 * divider so they don't blur into one block:
 *   - The header row (avatar + name + chevron) opens the profile
 *     dropdown (订阅 / 设置 / 退出)
 *   - The balance row is its own button — clicking anywhere on it
 *     opens the billing dialog directly, skipping the dropdown
 *
 * The 32px round avatar carries the Corivo amber gradient so it
 * tracks the theme automatically (light → amber-500, dark → amber-400).
 */
export function UserCard() {
  const { t } = useTranslation()
  const profile = useUserProfile()
  // Capability gates:
  //   * billing=false → 隐藏余额行 + 订阅菜单项 + 跳过 billingMe IPC
  //   * auth=false    → 隐藏退出菜单项（OSS 没有"登录"概念）
  // 没拿到 capabilities（首次渲染 race）保守地按 false 处理。
  const { data: capabilities } = useCapabilities()
  const billingEnabled = capabilities?.billing ?? false
  const authEnabled = capabilities?.auth ?? false
  const { data: billing } = useBillingMe({ enabled: billingEnabled })
  const balance = billing?.balance
  const balanceDisplay = balance == null ? "—" : `$${balance.toFixed(2)}`
  const displayName = profile.name || t.user.defaultName
  const initials = useMemo(() => initialsFor(displayName), [displayName])
  const [logoutOpen, setLogoutOpen] = useState(false)

  // Logout flow: dropdown action opens a confirm dialog (sign-out is
  // destructive — wipes the session token). The auth_logout command
  // also clears the cached creds; we then reset the profile store so
  // the sidebar redraws with the default display name, and the route
  // guard in app-boot will bounce to /login on next mount.
  const logoutMutation = useMutation({
    mutationFn: authLogout,
    onSuccess: () => {
      applyAuthStatus(null)
      toast.success(t.user.logoutSuccess)
      setLogoutOpen(false)
      // Hard reload to reset all in-memory caches (router state, react
      // query, etc.) — the closed-beta gate in AppBoot picks it up
      // from a fresh boot and lands on /login.
      window.location.reload()
    },
    onError: (error: unknown) => {
      toast.error(t.common.logoutFailed(String(error)))
    },
  })

  return (
    <>
      <div className="mx-1 flex flex-col rounded-md">
        <DropdownMenu>
          <DropdownMenuTrigger className="group flex items-center gap-3 rounded-t-md px-2 py-1.5 text-left outline-none transition-colors hover:bg-muted data-[state=open]:bg-muted">
            <Avatar
              imageUrl={profile.avatarUrl}
              initials={initials}
              alt={displayName}
            />

            <span className="min-w-0 flex-1 truncate font-display text-[13.5px] font-semibold tracking-[-0.005em] text-foreground">
              {displayName}
            </span>

            <ChevronDown
              aria-hidden="true"
              className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70 transition-colors group-hover:text-muted-foreground"
            />
          </DropdownMenuTrigger>

          <DropdownMenuContent
            side="top"
            align="start"
            sideOffset={8}
            className="w-56"
          >
            <DropdownMenuLabel className="flex items-center gap-2 px-2 py-1.5">
              <Avatar
                imageUrl={profile.avatarUrl}
                initials={initials}
                alt={displayName}
                size={28}
              />
              <span className="min-w-0 truncate text-[13px] font-medium text-foreground">
                {displayName}
              </span>
            </DropdownMenuLabel>
            {billingEnabled ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => openBillingDialog()}>
                  <CreditCard />
                  <span>{t.user.subscription}</span>
                </DropdownMenuItem>
              </>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => openSettingsDialog()}>
              <Settings />
              <span>{t.user.settings}</span>
            </DropdownMenuItem>
            {authEnabled ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={(e) => {
                    // Defer opening the dialog until after Radix has finished
                    // closing the dropdown — otherwise the focus ring snaps
                    // back to the trigger before the dialog mounts.
                    e.preventDefault()
                    setTimeout(() => setLogoutOpen(true), 0)
                  }}
                  className="text-destructive focus:text-destructive"
                >
                  <LogOut />
                  <span>{t.user.logout}</span>
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>

        {billingEnabled ? (
          <button
            type="button"
            onClick={() => openBillingDialog()}
            title={t.sidebar.balanceTitle}
            aria-label={t.sidebar.balanceTitle}
            className="group/balance flex items-center gap-3 rounded-b-md border-t border-border/40 px-2 py-1.5 text-left transition-colors hover:bg-muted"
          >
            {/* Spacer: 32px avatar width + 12px gap so the balance line
                starts at the same x as the displayName above. */}
            <span aria-hidden className="w-8 shrink-0" />
            <span className="min-w-0 flex-1 truncate text-[11.5px] tabular-nums text-muted-foreground">
              {balanceDisplay}
              <span className="ml-1 text-muted-foreground/60">USD</span>
            </span>
            <span className="shrink-0 font-mono text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--corivo-amber)] transition-colors group-hover/balance:text-[var(--corivo-amber-hover)]">
              {t.sidebar.balanceAction}
            </span>
          </button>
        ) : null}
      </div>

      <Dialog open={logoutOpen} onOpenChange={setLogoutOpen}>
        <DialogContent className="sm:max-w-[400px]">
          <DialogHeader>
            <DialogTitle className="font-display text-[17px] font-semibold tracking-[-0.015em]">
              {t.user.logoutConfirm.title}
            </DialogTitle>
            <DialogDescription className="text-[13px] leading-[1.55]">
              {t.user.logoutConfirm.description}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2 sm:gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => setLogoutOpen(false)}
              disabled={logoutMutation.isPending}
            >
              {t.common.cancel}
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => logoutMutation.mutate()}
              disabled={logoutMutation.isPending}
            >
              {logoutMutation.isPending
                ? t.user.logoutConfirm.pending
                : t.user.logoutConfirm.confirm}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

function Avatar({
  imageUrl,
  initials,
  alt,
  size = 32,
}: {
  imageUrl: string | null
  initials: string
  alt: string
  size?: number
}) {
  if (imageUrl) {
    return (
      <img
        src={imageUrl}
        alt={alt}
        width={size}
        height={size}
        className="shrink-0 rounded-full object-cover"
        style={{
          width: size,
          height: size,
          border: "0.5px solid var(--border)",
        }}
      />
    )
  }
  return (
    <span
      aria-hidden
      className="inline-flex shrink-0 items-center justify-center rounded-full font-display font-semibold uppercase text-white"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.36),
        background:
          "linear-gradient(135deg, color-mix(in oklab, var(--corivo-amber) 75%, white) 0%, var(--corivo-amber) 60%, color-mix(in oklab, var(--corivo-amber) 80%, black) 100%)",
        border: "0.5px solid var(--border)",
        textShadow: "0 1px 0 rgba(0, 0, 0, 0.08)",
      }}
    >
      {initials}
    </span>
  )
}

function initialsFor(name: string): string {
  const trimmed = name.trim()
  if (!trimmed) return "?"
  const parts = trimmed.split(/\s+/).filter(Boolean)
  if (parts.length >= 2) {
    return (parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")
  }
  return trimmed.slice(0, 2)
}
