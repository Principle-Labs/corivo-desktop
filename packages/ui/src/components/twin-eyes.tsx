import type { CSSProperties } from "react"
import { cn } from "../lib/utils"

type TwinEyesSize = "xs" | "sm" | "md" | "lg"
type TwinEyesState = "awake" | "sleepy"

type TwinEyesProps = {
  size?: TwinEyesSize
  animated?: boolean
  state?: TwinEyesState
  className?: string
}

const SIZE_MAP: Record<TwinEyesSize, { eye: number; gap: number; glow: number }> = {
  xs: { eye: 5, gap: 5, glow: 6 },
  sm: { eye: 7, gap: 5, glow: 10 },
  md: { eye: 14, gap: 8, glow: 16 },
  lg: { eye: 28, gap: 14, glow: 24 },
}

export function TwinEyes({
  size = "md",
  animated = true,
  state = "awake",
  className,
}: TwinEyesProps) {
  const { eye, gap, glow } = SIZE_MAP[size]
  const sleepy = state === "sleepy"

  const eyeStyle: CSSProperties = {
    width: `${eye}px`,
    height: `${eye}px`,
    borderRadius: "999px",
    background:
      "radial-gradient(circle at 35% 30%, #fbbf24, #d97706 70%)",
    boxShadow: `0 0 ${glow}px rgba(217, 119, 6, ${sleepy ? 0.25 : 0.55})`,
    display: "inline-block",
    flexShrink: 0,
    transformOrigin: "center",
    transform: sleepy ? "scaleY(0.18)" : undefined,
    opacity: sleepy ? 0.7 : 1,
  }

  const pairStyle: CSSProperties = {
    display: "inline-flex",
    alignItems: "center",
    gap: `${gap}px`,
  }

  return (
    <span
      aria-hidden="true"
      role="presentation"
      className={cn("corivo-eye-pair", className)}
      style={pairStyle}
    >
      <span
        className="corivo-eye"
        style={{
          ...eyeStyle,
          animation:
            animated && !sleepy
              ? "corivo-eye-breathe 3.4s ease-in-out infinite, corivo-eye-blink 5.2s ease-in-out infinite, corivo-eye-glance 6.4s ease-in-out infinite"
              : undefined,
        }}
      />
      <span
        className="corivo-eye"
        style={{
          ...eyeStyle,
          animation:
            animated && !sleepy
              ? "corivo-eye-breathe 3.4s ease-in-out infinite 80ms, corivo-eye-blink 5.2s ease-in-out infinite 80ms, corivo-eye-glance 6.4s ease-in-out infinite 80ms"
              : undefined,
        }}
      />
    </span>
  )
}
