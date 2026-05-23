import { useAppIcon } from "@/hooks/use-app-icon";
import { useTranslation } from "@/i18n";
import type { FocusContext, QuickAskSkeleton } from "@/lib/types";

interface FocusCardProps {
  focus: FocusContext | null;
  /** Available right after `window.show()` on the hotkey path; carries
   *  app/window/url so the title row renders before the AX walk lands. */
  skeleton?: QuickAskSkeleton | null;
  /** True while phase B (AX walk + adapter pipeline + frame ingest) is
   *  still running. With `skeleton` set we render the partial card
   *  (title row + preview shimmer); without it, the full loading
   *  skeleton — same as a cold direct-open mount. */
  loading: boolean;
  /** The user's *current* foreground app, when it has drifted away
   *  from the locked anchor. Drives the "+ 切换到 [App]" chip the user
   *  clicks to re-anchor the next message (CO-3 #3). `null` when there's
   *  no divergence to advertise. */
  liveFocus?: FocusContext | null;
  /** Click handler for the swap chip. Required when `liveFocus` is set. */
  onAdoptLive?: () => void;
}

export function FocusCard({
  focus,
  skeleton,
  loading,
  liveFocus,
  onAdoptLive,
}: FocusCardProps) {
  const { t } = useTranslation();
  // Partial-known state: skeleton arrived, AX walk still in flight.
  if (!focus && skeleton) {
    return (
      <div className="quick-ask-chips">
        <span className="quick-ask-chip quick-ask-chip--loading">
          <AppIcon bundleId={skeleton.app_bundle_id} />
          {skeleton.app_name ??
            skeleton.app_bundle_id ??
            t.quickAsk.focus.currentWindow}
        </span>
        {skeleton.window_title || skeleton.url ? (
          <span className="quick-ask-chip quick-ask-chip--loading">
            {skeleton.window_title ?? skeleton.url}
          </span>
        ) : null}
      </div>
    );
  }

  if (loading || !focus) {
    return (
      <div className="quick-ask-chips">
        <span className="quick-ask-chip quick-ask-chip--loading">
          {t.quickAsk.focus.readingWindow}
        </span>
      </div>
    );
  }

  if (focus.excluded) {
    return (
      <div className="quick-ask-chips">
        <span className="quick-ask-chip quick-ask-chip--warn">
          🔒{" "}
          {focus.app_name ??
            focus.app_bundle_id ??
            t.quickAsk.focus.currentApp}{" "}
          {t.quickAsk.focus.excludedSuffix}
        </span>
      </div>
    );
  }

  if (focus.empty) {
    return (
      <div className="quick-ask-chips">
        <span className="quick-ask-chip quick-ask-chip--warn">
          💭 {t.quickAsk.focus.cantReadWindow}
        </span>
      </div>
    );
  }

  const subtitle = pickSubtitle(focus);

  return (
    <div className="quick-ask-chips">
      <span className="quick-ask-chip">
        <AppIcon bundleId={focus.app_bundle_id} />
        {focus.app_name ??
          focus.app_bundle_id ??
          t.quickAsk.focus.currentWindow}
      </span>
      {subtitle ? <span className="quick-ask-chip">{subtitle}</span> : null}
      {liveFocus && onAdoptLive ? (
        <button
          type="button"
          className="quick-ask-chip quick-ask-chip--switch"
          onClick={onAdoptLive}
          title={t.quickAsk.focus.switchTitle}
        >
          <AppIcon bundleId={liveFocus.app_bundle_id} />
          {t.quickAsk.focus.switchTo(
            liveFocus.app_name ??
              liveFocus.app_bundle_id ??
              t.quickAsk.focus.currentApp,
          )}
        </button>
      ) : null}
    </div>
  );
}

/// Inline app-icon slot for the chips above. Reserves the 14px slot
/// the moment we know a bundle id (avoids layout shift while the IPC
/// resolves) and fades the PNG in once it lands. Renders nothing when
/// `bundleId` is null/undefined — keeps the chip plain text in the
/// rare case the foreground probe didn't return one.
function AppIcon({ bundleId }: { bundleId: string | null | undefined }) {
  const { data: iconUrl } = useAppIcon(bundleId);
  if (!bundleId) return null;
  return (
    <span className="quick-ask-chip__icon" aria-hidden="true">
      {iconUrl ? <img src={iconUrl} alt="" /> : null}
    </span>
  );
}

function pickSubtitle(focus: FocusContext): string | null {
  // Prefer adapter-specific keys when present; fall back to the
  // generic window_title / url surface. `page_title` comes from the
  // browser adapters (chrome / safari / arc) and has the app-name
  // suffix already stripped — without this branch we'd fall through
  // to focus.window_title and surface "...tab — Google Chrome".
  const payload = focus.adapter_payload;
  if (payload && typeof payload === "object") {
    const pageTitle = pickString(payload, "page_title");
    if (pageTitle) return pageTitle;
    const path = pickString(payload, "path") ?? pickString(payload, "file");
    if (path) return path;
    const url = pickString(payload, "url");
    if (url) return url;
    const channel = pickString(payload, "channel");
    if (channel) return `#${channel}`;
  }
  return focus.window_title ?? focus.url ?? null;
}

function pickString(obj: Record<string, unknown>, key: string): string | null {
  const value = obj[key];
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}
