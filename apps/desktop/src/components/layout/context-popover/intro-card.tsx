import { useEffect, useState } from "react";
import { ArrowUpRight, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useTranslation } from "@/i18n";

const DISMISSED_KEY = "corivo.context-popover.intro-dismissed";

/**
 * Link the "Learn more" arrow opens. Centralised here so the marketing
 * page rename only touches one constant; pulled out of i18n on purpose
 * because the URL is the same across locales (the page itself does
 * locale routing).
 */
const LEARN_MORE_URL = "https://corivo.ai/privacy/context-awareness";

/**
 * Closable card that introduces "context awareness" at the top of the
 * privacy popover. The user dismisses it once; from then on we never
 * render it again (localStorage-persisted).
 *
 * Why the dismiss is sticky: the popover is a high-frequency surface
 * (users open it whenever they want to pause / wipe / check status).
 * The intro card is value on the first few visits and noise after that.
 *
 * Why a card and not a tooltip: the framing matters. Showing "Corivo
 * remembers your work, no integrations needed" inside the privacy menu
 * is what makes the next rows (pause, delete) read as control over
 * something positive rather than damage limitation.
 */
export function ContextAwarenessIntroCard() {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(true); // SSR-safe default

  // Read the dismissed flag on mount. We default-true above so the
  // first render hides the card; a user who never dismissed it will
  // see it flash in on hydration, which is fine — the popover only
  // opens on click anyway, so there's no FOUC concern.
  useEffect(() => {
    try {
      setDismissed(window.localStorage.getItem(DISMISSED_KEY) === "1");
    } catch {
      // Private mode / sandboxed: just keep the default and move on.
    }
  }, []);

  if (dismissed) return null;

  const handleDismiss = () => {
    try {
      window.localStorage.setItem(DISMISSED_KEY, "1");
    } catch {
      // No-op if storage is unavailable.
    }
    setDismissed(true);
  };

  return (
    <div className="relative rounded-md border border-border bg-card/60 px-3 py-2.5">
      <button
        type="button"
        onClick={handleDismiss}
        aria-label={t.contextPopover.intro.dismissAria}
        className="absolute right-1.5 top-1.5 rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-muted hover:text-foreground"
      >
        <X className="h-3 w-3" />
      </button>
      <div className="pr-5 text-[12.5px] font-semibold leading-snug text-foreground/90">
        {t.contextPopover.intro.title}
      </div>
      <p className="mt-1 pr-5 text-[11.5px] leading-[1.5] text-muted-foreground">
        {t.contextPopover.intro.body}
      </p>
      <button
        type="button"
        onClick={() => {
          void openUrl(LEARN_MORE_URL);
        }}
        className="mt-1.5 inline-flex items-center gap-0.5 text-[11.5px] text-foreground/80 underline-offset-2 transition-colors hover:text-foreground hover:underline"
      >
        {t.contextPopover.intro.learnMore}
        <ArrowUpRight className="h-3 w-3" />
      </button>
    </div>
  );
}
