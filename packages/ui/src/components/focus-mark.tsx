import * as React from "react"
import { cn } from "../lib/utils"

/**
 * Focus Mark — Corivo's visual signature.
 *
 * A 4-corner viewfinder + center dot inside a 24×24 viewBox. Single
 * stroke-width 1.25 with rounded line caps, sharp 90° corners; the
 * "warmth" comes from the line-end radius and the small filled center
 * dot, the precision from the L-shapes.
 *
 * Uses `currentColor` so callers control hue with `color` /
 * `text-{token}` Tailwind utilities, e.g. `<FocusMark className="text-fg-muted" />`.
 *
 * Sizes (system-v0 §05): 16 / 20 / 24 / 32 / 48. The component takes a
 * `size` prop for convenience and falls back to width/height
 * className overrides if you need something off-scale.
 */
export interface FocusMarkProps extends React.SVGAttributes<SVGSVGElement> {
  /** Edge length in pixels. Default 24. */
  size?: number
}

export const FocusMark = React.forwardRef<SVGSVGElement, FocusMarkProps>(
  ({ size = 24, className, strokeWidth = 1.25, ...rest }, ref) => {
    return (
      <svg
        ref={ref}
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        className={cn("inline-block flex-shrink-0", className)}
        {...rest}
      >
        <path d="M5 9V5H9" />
        <path d="M15 5H19V9" />
        <path d="M19 15V19H15" />
        <path d="M9 19H5V15" />
        <circle cx="12" cy="12" r="1.25" fill="currentColor" stroke="none" />
      </svg>
    )
  },
)
FocusMark.displayName = "FocusMark"
