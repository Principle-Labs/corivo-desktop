import { useEffect, useMemo, useState } from "react"
import {
  CheckCircle2,
  Clock,
  ExternalLink,
  Loader2,
  RefreshCw,
  ShieldCheck,
  XCircle,
} from "lucide-react"
import { toast } from "sonner"
import { cn } from "@repo/ui/lib/utils"
import { Button } from "@repo/ui/components/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@repo/ui/components/dialog"
import type { BillingLastPayment } from "@corivo/shared-types"
import { useBillingMe } from "@/hooks/use-billing"
import { useTranslation, type LocaleDict } from "@/i18n"
import { billingStartCheckout } from "@/lib/tauri"
import {
  closeBillingDialog,
  setBillingDialogOpen,
  useBillingDialogOpen,
} from "@/stores/billing-dialog-store"

type StatusKey = "credited" | "paid" | "pending" | "failed"

const STATUS_VISUALS: Record<
  StatusKey,
  { icon: React.ReactNode; className: string }
> = {
  credited: {
    icon: <CheckCircle2 className="h-3 w-3" />,
    className: "text-emerald-500",
  },
  paid: {
    icon: <CheckCircle2 className="h-3 w-3" />,
    className: "text-emerald-500",
  },
  pending: {
    icon: <Clock className="h-3 w-3" />,
    className: "text-amber-500",
  },
  failed: {
    icon: <XCircle className="h-3 w-3" />,
    className: "text-red-500",
  },
}

// Backend returns three amounts in ascending order; we map by position
// to the localized tier presets in the active dictionary.
type TierKey = "starter" | "standard" | "longTerm"
const TIER_KEYS: TierKey[] = ["starter", "standard", "longTerm"]
const RECOMMENDED_TIER: TierKey = "standard"

function StatusBadge({
  status,
  t,
}: {
  status: string
  t: LocaleDict["billing"]["balance"]["status"]
}) {
  const visual = STATUS_VISUALS[status as StatusKey]
  const label =
    status === "credited"
      ? t.credited
      : status === "paid"
        ? t.paid
        : status === "pending"
          ? t.pending
          : status === "failed"
            ? t.failed
            : status
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1",
        visual?.className ?? "text-muted-foreground",
      )}
    >
      {visual?.icon}
      {label}
    </span>
  )
}

function LastPaymentLine({
  p,
  t,
  localeTag,
}: {
  p: BillingLastPayment
  t: LocaleDict["billing"]["balance"]
  localeTag: string
}) {
  const date = p.creditedAt
    ? new Date(p.creditedAt).toLocaleDateString(localeTag, {
        month: "long",
        day: "numeric",
      })
    : null
  return (
    <p className="mt-2.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
      <span>{t.lastPaymentPrefix(p.creditUsd)}</span>
      {date ? (
        <>
          <span className="text-muted-foreground/40">·</span>
          <span>{date}</span>
        </>
      ) : null}
      <span className="text-muted-foreground/40">·</span>
      <StatusBadge status={p.status} t={t.status} />
    </p>
  )
}

export function BillingDialog() {
  const open = useBillingDialogOpen()
  const { t, lang } = useTranslation()
  const tb = t.billing
  const localeTag = lang === "en" ? "en-US" : "zh-CN"

  const { data: me, isFetching, refetch } = useBillingMe()
  const [selected, setSelected] = useState<number | null>(null)
  const [submitting, setSubmitting] = useState(false)

  // Refetch the user's billing snapshot every time the dialog opens —
  // the most common open path is "user just paid via Stripe and came
  // back to confirm", so the cached value is usually stale.
  useEffect(() => {
    if (!open) return
    setSelected(null)
    void (async () => {
      try {
        await refetch({ throwOnError: true })
      } catch (err) {
        console.error("[billing-dialog.billing_me_failed]", err)
        toast.error(tb.toasts.loadFailed, {
          description: err instanceof Error ? err.message : String(err),
        })
      }
    })()
  }, [open, refetch, tb.toasts.loadFailed])

  const handleTopUp = async () => {
    if (selected == null) return
    setSubmitting(true)
    try {
      await billingStartCheckout(selected)
      toast(tb.toasts.openedBrowser, {
        description: tb.toasts.openedBrowserHint,
      })
    } catch (err) {
      console.error("[billing-dialog.start_checkout_failed]", err)
      toast.error(tb.toasts.checkoutFailed, {
        description: err instanceof Error ? err.message : String(err),
      })
    } finally {
      setSubmitting(false)
    }
  }

  const handleRefresh = async () => {
    try {
      await refetch({ throwOnError: true })
    } catch (err) {
      console.error("[billing-dialog.refresh_failed]", err)
      toast.error(tb.toasts.refreshFailed, {
        description: err instanceof Error ? err.message : String(err),
      })
    }
  }

  const balance = me?.balance
  const amounts = me?.allowedAmountsUsd ?? [20, 80, 200]
  const lastPayment = me?.lastPayment ?? null
  const loading = isFetching

  const selectedTierKey = useMemo<TierKey | null>(() => {
    if (selected == null) return null
    const idx = amounts.indexOf(selected)
    return idx >= 0 ? (TIER_KEYS[idx] ?? null) : null
  }, [selected, amounts])

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setBillingDialogOpen(true)
        } else {
          closeBillingDialog()
        }
      }}
    >
      <DialogContent className="max-w-md gap-0 overflow-hidden p-0">
        {/* ── Header ── */}
        <div className="px-6 pt-6 pb-5">
          <DialogTitle className="text-base font-semibold leading-none">
            {tb.title}
          </DialogTitle>
          <DialogDescription className="mt-2 text-xs leading-relaxed text-muted-foreground">
            {tb.description}
          </DialogDescription>
        </div>

        {/* ── Balance ── */}
        <div className="mx-6 mb-5 rounded-xl border border-border/60 bg-muted/30 px-5 py-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70">
                {tb.balance.label}
              </p>
              <div className="mt-1.5">
                {loading && balance == null ? (
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                ) : (
                  <span className="flex items-baseline gap-1.5">
                    <span className="text-[2rem] font-semibold leading-none tracking-tight tabular-nums">
                      {balance == null ? "—" : `$${balance.toFixed(2)}`}
                    </span>
                    <span className="text-sm text-muted-foreground">USD</span>
                  </span>
                )}
              </div>
              {lastPayment ? (
                <LastPaymentLine
                  p={lastPayment}
                  t={tb.balance}
                  localeTag={localeTag}
                />
              ) : null}
            </div>

            <button
              type="button"
              onClick={handleRefresh}
              disabled={loading}
              title={tb.balance.refresh}
              aria-label={tb.balance.refresh}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:opacity-40"
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </div>
        </div>

        {/* ── Tier picker + CTA ── */}
        <div className="px-6 pb-5">
          <p className="mb-3 text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70">
            {tb.picker.label}
          </p>
          <div className="grid grid-cols-3 gap-2">
            {amounts.map((amount, idx) => {
              const tierKey = TIER_KEYS[idx]
              const tier = tierKey ? tb.picker.tiers[tierKey] : null
              const isRecommended = tierKey === RECOMMENDED_TIER
              const isSelected = selected === amount
              return (
                <button
                  key={amount}
                  type="button"
                  onClick={() => setSelected(isSelected ? null : amount)}
                  className={cn(
                    "flex flex-col items-center justify-center gap-1.5 rounded-lg border py-3 text-center transition-all duration-150",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1",
                    isSelected
                      ? "border-foreground bg-foreground text-background shadow-sm"
                      : isRecommended
                        ? "border-foreground/40 bg-muted/40 text-foreground hover:border-foreground/60 hover:bg-muted/70"
                        : "border-border bg-muted/20 text-foreground hover:border-muted-foreground/40 hover:bg-muted/50",
                  )}
                >
                  {/* Marker row — visible "Recommended" pill on the
                      recommended tier; an invisible spacer of identical
                      size on the others, so all cards stay the same
                      height regardless of locale-dependent text width. */}
                  <span
                    aria-hidden={!isRecommended}
                    className={cn(
                      "rounded-full px-1.5 py-px text-[9px] font-semibold uppercase leading-none tracking-[0.1em]",
                      isRecommended
                        ? isSelected
                          ? "bg-background text-foreground"
                          : "bg-foreground text-background"
                        : "invisible",
                    )}
                  >
                    {tb.picker.recommendedBadge}
                  </span>

                  <span className="text-[1.0625rem] font-semibold leading-none tabular-nums">
                    ${amount}
                  </span>

                  {tier ? (
                    <span
                      className={cn(
                        "text-[10.5px] font-medium leading-none",
                        isSelected
                          ? "text-background/80"
                          : "text-muted-foreground",
                      )}
                    >
                      {tier.name}
                    </span>
                  ) : null}
                </button>
              )
            })}
          </div>

          <p className="mt-2.5 min-h-[1rem] text-center text-[11px] text-muted-foreground">
            {selectedTierKey
              ? tb.picker.tiers[selectedTierKey].duration
              : tb.picker.hint}
          </p>

          <Button
            className="mt-3 h-10 w-full gap-1.5 text-sm"
            disabled={selected == null || submitting}
            onClick={handleTopUp}
          >
            {submitting ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ExternalLink className="h-3.5 w-3.5" />
            )}
            {submitting
              ? tb.picker.cta.submitting
              : selected != null
                ? tb.picker.cta.ready(selected)
                : tb.picker.cta.idle}
          </Button>
        </div>

        {/* ── Usage breakdown ── */}
        <div className="border-t border-border/50 bg-muted/15 px-6 py-4">
          <p className="mb-2.5 text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70">
            {tb.usage.label}
          </p>
          <ul className="space-y-2">
            {(["ask", "quickAsk", "index"] as const).map((key) => {
              const item = tb.usage.items[key]
              return (
                <li key={key} className="flex items-baseline gap-2 text-xs">
                  <span className="mt-px h-1 w-1 shrink-0 translate-y-[5px] rounded-full bg-muted-foreground/40" />
                  <span className="leading-snug">
                    <span className="text-foreground/85">{item.label}</span>
                    <span className="text-muted-foreground"> · {item.hint}</span>
                  </span>
                </li>
              )
            })}
          </ul>
        </div>

        {/* ── Trust footer ── */}
        <div className="space-y-1 border-t border-border/50 px-6 py-4">
          <p className="flex items-center gap-1.5 text-[11px] text-foreground/75">
            <ShieldCheck className="h-3 w-3 shrink-0" />
            {tb.trust.primary}
          </p>
          <p className="pl-[18px] text-[11px] leading-relaxed text-muted-foreground/70">
            {tb.trust.secondary}
          </p>
        </div>
      </DialogContent>
    </Dialog>
  )
}
