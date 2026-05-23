import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import {
  Accessibility,
  ArrowRight,
  Check,
  ExternalLink,
  MonitorPlay,
} from "lucide-react";

import { cn } from "@repo/ui/lib/utils";

import { useTranslation } from "@/i18n";
import {
  checkAxPermission,
  checkScreenRecordingPermission,
  openAxSettings,
  openSystemSettingsPrivacy,
  requestScreenRecordingPermission,
} from "@/lib/tauri";

/**
 * Permission step — first of three onboarding screens (v3 redesign,
 * docs/design/onboarding-redesign-v2.html).
 *
 * Layout: single column, centered. Two permission rows stacked into
 * one card; each row has its own "授权" button (per the macOS rule
 * that AX + Screen Recording must be granted in two separate system
 * dialogs). Footer mono caption explains the auto-advance.
 *
 * Flow:
 *   1. User clicks "授权" on the AX row → kicks `openAxSettings()`
 *      (System Settings deep link). Button flips to "请求中…" and
 *      polls `check_ax_permission` until granted.
 *   2. Same for Screen Recording, independently.
 *   3. Once BOTH are granted, auto-advance to `/onboarding/demo`
 *      after 800 ms. Each row also surfaces a "打开系统设置" fallback
 *      after 6 s of being stuck in "asking" without the grant
 *      landing (system dialog dismissed / denied scenario).
 */

type RowPhase = "idle" | "asking" | "granted";

const POLL_MS = 1500;
const ADVANCE_AFTER_BOTH_MS = 800;
const STUCK_FALLBACK_MS = 6000;

/** Set by demo-step on first visit. If present here, the user came
 *  back via the ← back button, so we suppress auto-advance and show
 *  a manual "继续" button instead — otherwise auto-advance would
 *  immediately bounce them forward again (visible as a loop). */
const VISITED_DEMO_KEY = "corivo-onboarding-visited-demo";

export function PermissionStep() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  // Captured once on mount: are we landing here from a clean start
  // (false) or from demo-step's ← back button (true)? When true we
  // suppress the auto-advance so the user can actually stay on this
  // page; they advance forward via the manual "继续" button.
  const [returnedFromDemo] = useState(() => {
    if (typeof window === "undefined") return false;
    try {
      return sessionStorage.getItem(VISITED_DEMO_KEY) === "1";
    } catch {
      return false;
    }
  });

  const axQuery = useQuery({
    queryKey: ["onboarding-ax-permission"],
    queryFn: checkAxPermission,
    refetchInterval: POLL_MS,
  });
  const screenQuery = useQuery({
    queryKey: ["onboarding-screen-permission"],
    queryFn: checkScreenRecordingPermission,
    refetchInterval: POLL_MS,
  });

  const axGranted = axQuery.data ?? false;
  const screenGranted = screenQuery.data ?? false;
  const bothGranted = axGranted && screenGranted;

  const [axPhase, setAxPhase] = useState<RowPhase>("idle");
  const [screenPhase, setScreenPhase] = useState<RowPhase>("idle");

  // Drive each row's phase to "granted" the moment the polling
  // reports it. This handles both "user just clicked grant" and
  // "user lands here with the permission already granted" (e.g.
  // restart onboarding) — both end up with a green check.
  useEffect(() => {
    if (axGranted) {
      setAxPhase("granted");
    }
  }, [axGranted]);
  useEffect(() => {
    if (screenGranted) {
      setScreenPhase("granted");
    }
  }, [screenGranted]);

  // Auto-advance to demo once both grants land. 800 ms gives the user
  // a beat to register the green check before the page changes.
  //
  // Suppressed when `returnedFromDemo` is true — i.e. the user clicked
  // ← back inside demo-step. In that case the user is on this page
  // deliberately, so they get a manual "继续" button (rendered below)
  // instead of being pushed forward again.
  useEffect(() => {
    if (returnedFromDemo) return;
    if (!axGranted || !screenGranted) return;
    const timer = setTimeout(() => {
      void navigate({ to: "/onboarding/demo" });
    }, ADVANCE_AFTER_BOTH_MS);
    return () => clearTimeout(timer);
  }, [axGranted, screenGranted, returnedFromDemo, navigate]);

  const handleContinue = () => {
    void navigate({ to: "/onboarding/demo" });
  };

  const handleGrantAx = () => {
    if (axPhase !== "idle") return;
    setAxPhase("asking");
    void openAxSettings();
  };

  const handleGrantScreen = () => {
    if (screenPhase !== "idle") return;
    setScreenPhase("asking");
    // Already-denied case → the macOS call is a no-op and never
    // resolves a "granted" event back to us; the row's stuck-fallback
    // hint surfaces a System Settings deep link after STUCK_FALLBACK_MS.
    void requestScreenRecordingPermission().catch(() => {});
  };

  const statusLabels = {
    pending: t.onboarding.permission.status.pending,
    asking: t.onboarding.permission.status.asking,
    granted: t.onboarding.permission.status.granted,
  };

  return (
    <div className="relative flex h-full w-full flex-col items-center justify-center gap-10 py-8">
      {/* Soft amber glow background — matches the design mockup. */}
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            "radial-gradient(circle at 50% 50%, color-mix(in oklab, var(--corivo-amber) 14%, transparent) 0%, transparent 65%)",
        }}
      />

      <div className="relative flex max-w-[520px] flex-col items-center gap-3.5 text-center">
        <h1 className="font-display text-[34px] font-semibold leading-[1.1] tracking-[-0.028em] text-foreground sm:text-[38px]">
          {t.onboarding.permission.title}
        </h1>
        <p className="text-[14.5px] leading-[1.6] text-muted-foreground sm:text-[15px]">
          {t.onboarding.permission.subtitle}
        </p>
      </div>

      <div className="relative flex w-full max-w-[460px] flex-col gap-6">
        <div
          className="flex flex-col overflow-hidden rounded-[12px] border border-border bg-card"
          style={{ boxShadow: "var(--shadow-sm)" }}
        >
          <PermissionRow
            icon={<Accessibility className="h-[18px] w-[18px]" />}
            name={t.onboarding.permission.ax.name}
            desc={t.onboarding.permission.ax.desc}
            phase={axPhase}
            granted={axGranted}
            grantLabel={t.onboarding.permission.grant}
            statusLabels={statusLabels}
            onGrant={handleGrantAx}
            onOpenSettings={() => void openAxSettings()}
            deniedHint={t.onboarding.permission.denied}
            openSettingsLabel={t.onboarding.permission.openSettings}
          />
          <div className="border-t border-border" />
          <PermissionRow
            icon={<MonitorPlay className="h-[18px] w-[18px]" />}
            name={t.onboarding.permission.screen.name}
            desc={t.onboarding.permission.screen.desc}
            phase={screenPhase}
            granted={screenGranted}
            grantLabel={t.onboarding.permission.grant}
            statusLabels={statusLabels}
            onGrant={handleGrantScreen}
            onOpenSettings={() => void openSystemSettingsPrivacy()}
            deniedHint={t.onboarding.permission.denied}
            openSettingsLabel={t.onboarding.permission.openSettings}
          />
        </div>

        <p className="whitespace-pre-line text-center font-mono text-[10.5px] uppercase leading-[1.7] tracking-[0.14em] text-muted-foreground">
          {t.onboarding.permission.foot}
        </p>

        {/* Manual "继续" button — only when the user came back via
            demo-step's ← back and both grants are still in place. The
            normal first-run flow auto-advances and never renders this. */}
        {returnedFromDemo && bothGranted ? (
          <div className="flex justify-center">
            <button
              type="button"
              onClick={handleContinue}
              className="inline-flex h-12 items-center justify-center gap-2 rounded-[10px] bg-[var(--primary)] px-7 text-[14.5px] font-medium tracking-[-0.005em] text-[var(--primary-foreground)] transition-colors hover:bg-[var(--corivo-amber-hover)]"
              style={{ boxShadow: "var(--shadow-sm)" }}
            >
              {t.onboarding.nav.continue}
              <ArrowRight className="h-4 w-4" />
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

interface PermissionRowProps {
  icon: React.ReactNode;
  name: string;
  desc: string;
  phase: RowPhase;
  granted: boolean;
  grantLabel: string;
  statusLabels: { pending: string; asking: string; granted: string };
  onGrant: () => void;
  onOpenSettings: () => void;
  deniedHint: string;
  openSettingsLabel: string;
}

function PermissionRow({
  icon,
  name,
  desc,
  phase,
  granted,
  grantLabel,
  statusLabels,
  onGrant,
  onOpenSettings,
  deniedHint,
  openSettingsLabel,
}: PermissionRowProps) {
  // Surface a "the system dialog isn't going to pop again" hint when
  // the user is stuck in `asking` without the grant landing — covers
  // both "they dismissed the dialog" and "macOS refuses to re-prompt
  // because it was denied previously".
  const [showFallback, setShowFallback] = useState(false);
  useEffect(() => {
    if (phase !== "asking" || granted) {
      setShowFallback(false);
      return;
    }
    const t = setTimeout(() => setShowFallback(true), STUCK_FALLBACK_MS);
    return () => clearTimeout(t);
  }, [phase, granted]);

  return (
    <div>
      <div className="flex items-center gap-3.5 px-5 py-4">
        <span className="flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center text-muted-foreground">
          {icon}
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[14px] font-medium tracking-[-0.005em] text-foreground">
            {name}
          </span>
          <span className="text-[12.5px] leading-[1.4] text-muted-foreground">
            {desc}
          </span>
        </div>
        {granted ? (
          <span
            className="inline-flex flex-shrink-0 items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-[0.08em]"
            style={{ color: "var(--success)" }}
          >
            <Check className="h-3.5 w-3.5" />
            {statusLabels.granted}
          </span>
        ) : (
          <button
            type="button"
            onClick={onGrant}
            disabled={phase === "asking"}
            className={cn(
              "inline-flex h-8 flex-shrink-0 items-center justify-center rounded-[8px] px-4 text-[12.5px] font-medium tracking-[-0.005em] transition-colors",
              "bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--corivo-amber-hover)]",
              "disabled:cursor-default disabled:opacity-65 disabled:hover:bg-[var(--primary)]",
            )}
            style={{ boxShadow: "var(--shadow-sm)" }}
          >
            {phase === "asking" ? statusLabels.asking : grantLabel}
          </button>
        )}
      </div>

      {showFallback ? (
        <div className="flex items-center gap-3 border-t border-border bg-muted/40 px-5 py-2.5 text-[11.5px] leading-[1.5] text-muted-foreground">
          <span className="flex-1">{deniedHint}</span>
          <button
            type="button"
            onClick={onOpenSettings}
            className="inline-flex h-6 flex-shrink-0 items-center gap-1 rounded-md border border-[var(--border-strong)] bg-card px-2 text-[10.5px] hover:bg-background"
          >
            <ExternalLink className="h-2.5 w-2.5" />
            {openSettingsLabel}
          </button>
        </div>
      ) : null}
    </div>
  );
}
