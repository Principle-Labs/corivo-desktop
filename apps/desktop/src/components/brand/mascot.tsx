import corivoMascot from "@/assets/corivo-mascot.svg";
import { cn } from "@repo/ui/lib/utils";

/**
 * Brand mascot — wraps the canonical `corivo-mascot.svg` asset.
 *
 * The mascot is the only brand mark we use in product chrome (sidebar
 * brand lockup, login hero, onboarding step header, app boot splash,
 * AI message bubble). Always use this component instead of importing
 * the SVG directly so size + animation stay consistent across surfaces.
 *
 * Sizes line up with `auth-onboarding-v0.html` § Mascot:
 *   xs   18×23   inline brand lockup, sidebar progress bars
 *   sm   24×31   small lockup
 *   md   36×46   medium accent (AI message bubble header)
 *   lg   60×77   try-it step accent
 *   xl   88×112  login hero, onboarding hero
 *   2xl  120×154 splash / extra-large hero
 *
 * `breath` enables the "breathing" scale animation for waiting /
 * standby states (login waiting, app boot splash). `entrance` enables
 * the bouncy momoEntrance animation for first-paint moments.
 */
export type MascotSize = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

const SIZE_PX: Record<MascotSize, number> = {
  xs: 18,
  sm: 24,
  md: 36,
  lg: 60,
  xl: 88,
  "2xl": 120,
};

// SVG viewBox is 635×827 → height/width = 1.302
const ASPECT_HEIGHT_OVER_WIDTH = 827 / 635;

interface MascotProps {
  size?: MascotSize;
  /** Subtle breathing scale animation (waiting / standby chrome). */
  breath?: boolean;
  /** Bouncy entrance (used in onboarding hero, login hero). */
  entrance?: boolean;
  /** Continuous gentle float (used standalone in big hero contexts). */
  float?: boolean;
  className?: string;
}

export function Mascot({
  size = "md",
  breath = false,
  entrance = false,
  float = false,
  className,
}: MascotProps) {
  const widthPx = SIZE_PX[size];
  const heightPx = Math.round(widthPx * ASPECT_HEIGHT_OVER_WIDTH);

  return (
    <img
      src={corivoMascot}
      alt=""
      aria-hidden="true"
      draggable={false}
      width={widthPx}
      height={heightPx}
      className={cn(
        "select-none",
        entrance && "animate-momo-entrance",
        float && "animate-momo-float",
        breath && "[animation:corivo-eye-breathe_2.4s_ease-in-out_infinite]",
        className,
      )}
      style={{ width: widthPx, height: heightPx }}
    />
  );
}
