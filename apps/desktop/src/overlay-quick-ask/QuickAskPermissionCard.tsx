import { useTranslation } from "@/i18n";
import { execAgentPermissionReply } from "@/lib/tauri";
import type { PermissionRequest } from "@/hooks/use-permission-listener";

interface Props {
  request: PermissionRequest;
  onResolved: (id: string) => void;
}

/** Inline permission card surfaced inside the Quick Ask panel.
 *
 *  Lives in QA itself (rather than the main window's modal `Dialog`)
 *  so users who triggered an agent turn from QA can approve / deny
 *  without losing focus on the small overlay panel.
 *
 *  Reuses the same `execAgentPermissionReply` IPC and the same
 *  `t.ask.permission.*` strings as the main-window `PermissionDialog`
 *  — only the chrome differs. */
export function QuickAskPermissionCard({ request, onResolved }: Props) {
  const { t } = useTranslation();

  const reply = async (allow: boolean) => {
    try {
      await execAgentPermissionReply({ requestId: request.id, allow });
    } catch (e) {
      console.error("[quick-ask] execAgentPermissionReply failed", e);
    } finally {
      onResolved(request.id);
    }
  };

  const hasDetails =
    request.details && Object.keys(request.details).length > 0;

  return (
    <div className="quick-ask-permission" role="dialog" aria-modal="true">
      <div className="quick-ask-permission__title">
        {t.ask.permission.title}
      </div>
      <div className="quick-ask-permission__row">
        <span className="quick-ask-permission__label">
          {t.ask.permission.action}
        </span>
        <span className="quick-ask-permission__tool">{request.action}</span>
      </div>
      {request.reason && (
        <p className="quick-ask-permission__reason">{request.reason}</p>
      )}
      {hasDetails && (
        <details className="quick-ask-permission__details">
          <summary className="quick-ask-permission__details-summary">
            {t.ask.permission.viewDetails}
          </summary>
          <pre className="quick-ask-permission__args">
            {JSON.stringify(request.details, null, 2)}
          </pre>
        </details>
      )}
      <p className="quick-ask-permission__hint">{t.ask.permission.hint}</p>
      <div className="quick-ask-permission__actions">
        <button
          type="button"
          className="quick-ask-permission__btn quick-ask-permission__btn--deny"
          onClick={() => void reply(false)}
        >
          {t.ask.permission.deny}
        </button>
        <button
          type="button"
          className="quick-ask-permission__btn quick-ask-permission__btn--allow"
          onClick={() => void reply(true)}
        >
          {t.ask.permission.allow}
        </button>
      </div>
    </div>
  );
}
