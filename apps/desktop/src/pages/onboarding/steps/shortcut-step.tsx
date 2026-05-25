import { useNavigate } from "@tanstack/react-router";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowLeft } from "lucide-react";

import { cn } from "@repo/ui/lib/utils";

import { useTranslation } from "@/i18n";
import { fromInvokeError, markOnboardingCompleted } from "@/lib/tauri";

/**
 * Shortcut step — final onboarding screen (v3 redesign,
 * docs/design/onboarding-redesign-v2.html).
 *
 * Static teaching:
 *   1. Quick Ask popup preview (visual anchor for "what shows up")
 *   2. Amber chip "连按 2 下 ⌥" with vertical hairlines top + bottom
 *   3. Simplified Mac keyboard bottom row — fn ⌃ ⌥ ⌘ space ⌘ ⌥
 *      with both ⌥ keys amber-highlighted. No animations.
 *
 * The user clicks "进入 Corivo" to actually close out onboarding:
 *   - mark_onboarding_completed flips `onboarding_completed=true`,
 *     bumps `onboarding_version`, clears `onboarding_step`, and
 *     flips `first_quick_ask_done` (so the first real ⌥⌥ summon
 *     goes straight to the empty state).
 *   - On success we navigate to `/` (which redirects to /timeline).
 */
export function ShortcutStep() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const shortcutKey = platformQuickAskKey();
  const isWindows = shortcutKey === "Alt";

  const completeMutation = useMutation({
    mutationFn: markOnboardingCompleted,
    onSuccess: () => {
      void navigate({ to: "/", replace: true });
    },
    onError: (error: unknown) => {
      toast.error(t.common.saveFailed(fromInvokeError(error)));
    },
  });

  const handleBack = () => {
    if (completeMutation.isPending) return;
    void navigate({ to: "/onboarding/demo" });
  };

  const handleEnter = () => {
    if (completeMutation.isPending) return;
    completeMutation.mutate();
  };

  return (
    <div className="relative flex h-full w-full flex-col items-center justify-center gap-12 py-8">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            "radial-gradient(circle at 50% 50%, color-mix(in oklab, var(--corivo-amber) 14%, transparent) 0%, transparent 65%)",
        }}
      />

      <div className="relative flex w-full max-w-[720px] flex-col items-center gap-3.5 text-center">
        <h2 className="font-display text-[30px] font-semibold leading-[1.1] tracking-[-0.028em] text-foreground sm:text-[34px]">
          {t.onboarding.shortcut.title}
        </h2>
        <p className="text-[14.5px] leading-[1.6] text-muted-foreground sm:text-[15px]">
          {t.onboarding.shortcut.subtitle}
        </p>
      </div>

      {/* cue chip ↓ keyboard. No connecting hairline: only the LEFT
          Option key is amber, so a centered drop-line would point
          at the spacebar instead and read as misdirection. The
          chip's amber border + the amber Option key share the
          same hue — that color match is the visual link. */}
      <div className="relative flex flex-col items-center">
        <div
          className="mb-6 rounded-full border px-4 py-1.5 font-mono text-[11px] uppercase tracking-[0.08em]"
          style={{
            background: "var(--card)",
            borderColor: "var(--corivo-amber)",
            color: "var(--accent-foreground)",
            boxShadow: "var(--shadow-sm)",
          }}
        >
          {t.onboarding.shortcut.cue} {shortcutKey}
        </div>

        {/* Simplified keyboard bottom row. */}
        <div
          className="rounded-[18px] border p-6"
          style={{
            background: "var(--card)",
            borderColor: "var(--border-strong)",
            boxShadow: "var(--shadow-lg)",
          }}
        >
          {/* Highlight only the LEFT Option key. Highlighting both
              reads as "press them simultaneously" — but the gesture
              is "double-tap one Option". One amber key, one focal
              point. */}
          <div className="flex gap-[5px]">
            {isWindows ? (
              <>
                <Key>Ctrl</Key>
                <Key>Win</Key>
                <Key opt>Alt</Key>
                <Key wide={5}>&nbsp;</Key>
                <Key>Alt</Key>
                <Key>Win</Key>
                <Key>Ctrl</Key>
              </>
            ) : (
              <>
                <Key>fn</Key>
                <Key>⌃</Key>
                <Key opt>⌥</Key>
                <Key wide={1.25}>⌘</Key>
                <Key wide={5}>&nbsp;</Key>
                <Key wide={1.25}>⌘</Key>
                <Key>⌥</Key>
              </>
            )}
          </div>
        </div>
      </div>

      <div className="relative flex items-center gap-5">
        <button
          type="button"
          onClick={handleBack}
          disabled={completeMutation.isPending}
          className="inline-flex items-center gap-1.5 text-[13px] text-muted-foreground transition-colors hover:text-foreground disabled:opacity-65 disabled:hover:text-muted-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          {t.onboarding.nav.back}
        </button>
        <button
          type="button"
          onClick={handleEnter}
          disabled={completeMutation.isPending}
          className={cn(
            "inline-flex h-12 items-center justify-center rounded-[10px] px-8 text-[14.5px] font-medium tracking-[-0.005em] transition-colors",
            "bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--corivo-amber-hover)]",
            "disabled:cursor-default disabled:opacity-65",
          )}
          style={{ boxShadow: "var(--shadow-sm)" }}
        >
          {t.onboarding.shortcut.cta}
        </button>
      </div>
    </div>
  );
}

function platformQuickAskKey() {
  if (typeof navigator !== "undefined" && /win/i.test(navigator.platform)) {
    return "Alt";
  }
  return "⌥";
}

const KEY_UNIT = 38; // px
const KEY_GAP = 5; // px

function Key({
  children,
  wide = 1,
  opt = false,
}: {
  children: React.ReactNode;
  wide?: number;
  opt?: boolean;
}) {
  const width = wide * KEY_UNIT + (wide - 1) * KEY_GAP;
  const isWideLabel = typeof children === "string" && children.length > 1;

  return (
    <div
      className={cn(
        "grid place-items-center rounded-[6px] border font-mono",
        opt
          ? "text-[var(--primary-foreground)]"
          : "text-muted-foreground opacity-85",
      )}
      style={{
        height: KEY_UNIT,
        width,
        background: opt ? "var(--corivo-amber)" : "var(--background)",
        borderColor: opt ? "var(--corivo-amber)" : "var(--border-strong)",
        fontSize: opt ? 16 : isWideLabel ? 9 : 11,
        fontWeight: opt ? 600 : 400,
        letterSpacing: isWideLabel ? "0.04em" : undefined,
        boxShadow: opt
          ? "inset 0 -3px 0 color-mix(in oklab, var(--corivo-amber) 50%, black), 0 0 0 3px color-mix(in oklab, var(--corivo-amber) 20%, transparent), var(--shadow-md)"
          : "inset 0 -2px 0 color-mix(in oklab, var(--foreground) 10%, transparent), var(--shadow-sm)",
      }}
    >
      {children}
    </div>
  );
}
