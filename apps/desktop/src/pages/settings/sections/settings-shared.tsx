import type { ReactNode } from "react";
import { cn } from "@repo/ui/lib/utils";

/**
 * Section header at the top of every Settings tab. Uses display font
 * for the title, smaller muted body for the lede. Matches
 * docs/design/screens-v0.html §05 sc-header.
 */
export function SectionHeader({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="mb-6">
      <h2 className="font-display text-[20px] font-semibold leading-tight tracking-[-0.015em] text-foreground">
        {title}
      </h2>
      <p className="mt-1 text-[13px] leading-[1.55] text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

/**
 * Settings section group — small mono uppercase title, content
 * directly underneath, hairline divider on top (skipped for the
 * first one). Matches screens-v0 §05 sc-section.
 *
 * No card chrome; "important" content inside uses its own
 * hairline-bordered surface (e.g. the Inspector).
 */
export function FieldGroup({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("first:pt-0 first:border-t-0 border-t border-border pt-5 pb-5", className)}>
      <div className="mb-3.5 font-mono text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
        {title}
      </div>
      <div className="flex flex-col gap-3">{children}</div>
    </section>
  );
}

/**
 * Setting row — label / description on the left, control on the
 * right. Uses a pill-style toggle (matches screens-v0 §05 .toggle)
 * instead of a default checkbox.
 */
export function ToggleRow({
  label,
  description,
  value,
  onChange,
}: {
  label: string;
  description: string;
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-border pb-3 last:border-b-0 last:pb-0">
      <div className="flex-1 space-y-0.5">
        <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
          {label}
        </div>
        <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
          {description}
        </div>
      </div>
      <Toggle value={value} onChange={onChange} />
    </div>
  );
}

/**
 * Pill-style switch — macOS-flavored.
 *
 * Track is 28×16 (slightly smaller than the iOS 36×22 default — the
 * settings rows are dense and the larger pill made every line look
 * like a toggle). OFF track uses a `color-mix` against `--foreground`
 * so it stays clearly distinct from both `--muted` (page) and
 * `--card` (thumb), regardless of theme. ON track flips to
 * `--foreground`. Knob is fixed 12×12 with a 2px inset.
 */
export function Toggle({
  value,
  onChange,
  ariaLabel,
}: {
  value: boolean;
  onChange: (value: boolean) => void;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={ariaLabel}
      onClick={() => onChange(!value)}
      className={cn(
        "relative inline-flex h-[18px] w-[30px] shrink-0 items-center rounded-full transition-colors duration-150 ease-out",
        value
          ? "bg-foreground"
          : "bg-[color-mix(in_oklab,var(--foreground)_18%,transparent)]",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute h-[14px] w-[14px] rounded-full bg-white transition-transform duration-150 ease-out",
          value ? "translate-x-[14px]" : "translate-x-[2px]",
        )}
        style={{
          boxShadow:
            "0 1px 2px rgba(0, 0, 0, 0.20), 0 0 0 0.5px rgba(0, 0, 0, 0.06)",
        }}
      />
    </button>
  );
}

export function SettingsSkeleton() {
  return (
    <div className="space-y-6">
      {[1, 2, 3].map((item) => (
        <section key={item} className="border-t border-border pt-5">
          <div className="mb-3 h-3 w-20 animate-pulse rounded bg-muted" />
          <div className="space-y-2">
            <div className="h-9 animate-pulse rounded bg-muted/70" />
            <div className="h-9 animate-pulse rounded bg-muted/70" />
          </div>
        </section>
      ))}
    </div>
  );
}
