import { useQuery } from "@tanstack/react-query";

import { Label } from "@repo/ui/components/label";
import { useTranslation, type LocaleDict } from "@/i18n";
import type { HotkeyBinding } from "@/lib/types";
import { getHotkeyStatus } from "@/lib/tauri";

import { FieldGroup, SectionHeader } from "./settings-shared";

const HOTKEY_KEY = ["hotkey-status"] as const;

/**
 * Quick Ask — its own settings tab so the floating panel reads as a
 * first-class surface, not a footnote of the capture page.
 *
 * Today this section holds:
 *   1. A pitch card explaining what Quick Ask is (so first-time
 *      visitors land on the concept, not just the mechanic).
 *   2. The hotkey binding card (was previously buried under
 *      "上下文共享").
 *
 * Future additions live here — appearance toggles, focus-anchor
 * preferences, alternate bindings, etc.
 */
export function QuickAskSection() {
  const { t } = useTranslation();
  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.quickAsk.title}
        description={t.settings.quickAsk.description}
      />

      <PitchCard />
      <HotkeyCard />
    </div>
  );
}

function PitchCard() {
  const { t } = useTranslation();
  return (
    <div className="rounded-[10px] border border-border bg-[color-mix(in_oklab,var(--accent)_55%,var(--card))] px-5 py-4">
      <div className="flex items-center gap-2">
        <KbdKey>⌥</KbdKey>
        <KbdKey>⌥</KbdKey>
        <span className="ml-1 text-[13px] font-semibold tracking-[-0.005em] text-foreground">
          {t.settings.quickAsk.pitchTitle}
        </span>
      </div>
      <p className="mt-2 text-[12.5px] leading-[1.6] text-muted-foreground">
        {t.settings.quickAsk.pitchBody}
      </p>
    </div>
  );
}

function HotkeyCard() {
  const { t } = useTranslation();
  const status = useQuery({
    queryKey: HOTKEY_KEY,
    queryFn: getHotkeyStatus,
  });
  const data = status.data;

  return (
    <FieldGroup title={t.settings.quickAsk.hotkeyGroup}>
      <div className="flex items-center justify-between gap-4">
        <Label className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
          {t.settings.quickAsk.hotkey.currentLabel}
        </Label>
        <span className="font-mono text-[12px] text-muted-foreground">
          {status.isLoading
            ? "…"
            : data
              ? bindingLabel(data.binding, t)
              : t.common.unknown}
        </span>
      </div>
      <p className="text-[11.5px] leading-[1.55] text-muted-foreground">
        {t.settings.quickAsk.hotkey.explanation}
      </p>
      {data?.error ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-[11.5px] leading-[1.5] text-destructive">
          {data.error}
        </div>
      ) : null}
      {data && !data.error && !data.installed ? (
        <div className="rounded-md border border-primary/40 bg-accent px-3 py-2 text-[11.5px] leading-[1.5] text-accent-foreground">
          {t.settings.quickAsk.hotkey.notInstalled}
        </div>
      ) : null}
    </FieldGroup>
  );
}

function bindingLabel(binding: HotkeyBinding, t: LocaleDict): string {
  switch (binding) {
    case "double_tap_option":
      return t.settings.quickAsk.hotkey.bindings.doubleTapOption;
    default: {
      const exhaustive: never = binding;
      return exhaustive;
    }
  }
}

function KbdKey({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex h-[20px] min-w-[20px] items-center justify-center rounded-[4px] border border-border bg-card px-[6px] font-mono text-[11px] font-medium text-foreground shadow-[0_1px_0_rgba(28,26,23,0.04)]">
      {children}
    </kbd>
  );
}
