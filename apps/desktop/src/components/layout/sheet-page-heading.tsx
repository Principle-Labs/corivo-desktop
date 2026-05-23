import type { ReactNode } from "react";

import { cn } from "@repo/ui/lib/utils";

export interface SheetPageHeadingProps {
  /**
   * Small mono uppercase kicker shown above the title, with an amber
   * status dot. Typically a page kind + count, e.g. "Knowledge · 62 命题".
   */
  kicker: ReactNode;
  /**
   * Large Fraunces headline. Pass inline `<em>` spans to get the italic
   * amber emphasis that the design brief uses for the accent word.
   */
  title: ReactNode;
  /**
   * Optional Caveat handwritten subtitle — Corivo's "voice" under the
   * headline. Rotates slightly so it reads like a margin note.
   */
  subtitle?: ReactNode;
  /**
   * When true, shrinks the heading so it can sit above a scrollable
   * region without dominating the viewport. Pure CSS transition — the
   * caller decides when to flip this (e.g. on inner-list scroll).
   */
  compact?: boolean;
}

/**
 * Shared Fraunces + Caveat page heading used on both the Today
 * overview and the Knowledge page. Keeps kicker / headline /
 * handwritten-subtitle typography consistent across the product
 * without baking copy into the component.
 */
export function SheetPageHeading({
  kicker,
  title,
  subtitle,
  compact = false,
}: SheetPageHeadingProps) {
  return (
    <div
      className={cn(
        "flex flex-col transition-[gap] duration-200 ease-out",
        compact ? "gap-1.5" : "gap-4",
      )}
    >
      <div
        className={cn(
          "font-mono uppercase tracking-[0.18em] text-primary transition-[font-size,opacity] duration-200 ease-out",
          compact ? "text-[10px] opacity-75" : "text-[11px]",
        )}
      >
        <span
          aria-hidden
          className="mr-2.5 inline-block h-1.5 w-1.5 rounded-full align-middle"
          style={{
            background: "var(--corivo-amber)",
            boxShadow:
              "0 0 8px color-mix(in oklab, var(--corivo-amber) 55%, transparent)",
          }}
        />
        {kicker}
      </div>
      <h1
        className={cn(
          "font-display font-semibold leading-[1.05] tracking-[-0.025em] text-foreground transition-[font-size] duration-200 ease-out",
          compact ? "text-2xl lg:text-3xl" : "text-4xl lg:text-5xl",
        )}
      >
        {title}
      </h1>
      {subtitle && !compact && (
        <p className="max-w-[520px] text-base leading-[1.55] text-muted-foreground">
          {subtitle}
        </p>
      )}
    </div>
  );
}
