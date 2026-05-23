import { Button } from "@repo/ui/components/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@repo/ui/components/dialog";

import { useTranslation } from "@/i18n";
import { execAgentPermissionReply } from "@/lib/tauri";
import type { PermissionRequest } from "@/hooks/use-permission-listener";

interface PermissionDialogProps {
  request: PermissionRequest | null;
  onResolved: (id: string) => void;
}

export function PermissionDialog({ request, onResolved }: PermissionDialogProps) {
  const { t } = useTranslation();
  const reply = async (allow: boolean) => {
    if (!request) return;
    try {
      await execAgentPermissionReply({ requestId: request.id, allow });
    } catch (e) {
      console.error("execAgentPermissionReply failed", e);
    } finally {
      onResolved(request.id);
    }
  };

  return (
    <Dialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open && request) {
          // Closing the dialog any other way (Escape / overlay click)
          // counts as deny — never leave the agent hanging on a reply.
          void reply(false);
        }
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.ask.permission.title}</DialogTitle>
        </DialogHeader>
        {request && (
          <div className="space-y-3 text-sm">
            <div>
              <span className="text-muted-foreground">
                {t.ask.permission.action}
              </span>
              <span className="ml-2 font-medium text-foreground">
                {request.action}
              </span>
            </div>
            {request.reason && (
              <div>
                <span className="text-muted-foreground">
                  {t.ask.permission.reason}
                </span>
                <p className="mt-1 text-foreground">{request.reason}</p>
              </div>
            )}
            {request.details && Object.keys(request.details).length > 0 && (
              <details className="group">
                <summary className="cursor-pointer list-none text-xs text-muted-foreground hover:text-foreground select-none">
                  <span className="inline-block transition-transform group-open:rotate-90">
                    ›
                  </span>{" "}
                  {t.ask.permission.viewDetails}
                </summary>
                <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-muted p-2 font-mono text-xs">
                  {JSON.stringify(request.details, null, 2)}
                </pre>
              </details>
            )}
            <p className="text-xs text-muted-foreground">
              {t.ask.permission.hint}
            </p>
          </div>
        )}
        <DialogFooter className="gap-2">
          <Button variant="outline" onClick={() => void reply(false)}>
            {t.ask.permission.deny}
          </Button>
          <Button onClick={() => void reply(true)}>
            {t.ask.permission.allow}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
