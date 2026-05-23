import { useState } from "react";
import { AlertTriangle } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@repo/ui/components/dialog";
import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";

import { useDataDeleteRange } from "@/hooks/use-capture";
import { useTranslation } from "@/i18n";

type Unit = "minutes" | "hours" | "days";

const UNIT_TO_SECONDS: Record<Unit, number> = {
  minutes: 60,
  hours: 60 * 60,
  days: 24 * 60 * 60,
};

const CONFIRM_PHRASE = "delete";

/**
 * Custom-range delete confirmation. Mirrors Littlebird's two-input
 * verification: pick a duration, then type "delete" to confirm. Both
 * gates are required before the destructive button is enabled.
 *
 * Why a duration + unit picker instead of a free-form datetime: the
 * mental model the popover sets up is "delete the last X" — past-window
 * deletes don't need calendar precision, and a unit dropdown is fast on
 * keyboard ("5", Tab, m, Enter). Calendar pickers in a privacy modal
 * also encourage the wrong question ("when did I look at the bad thing"
 * vs. "how recently could it have leaked").
 *
 * The dialog is fully controlled. The parent (`ContextPopover`) opens
 * it via `open` and resets local state on close.
 */
export function DeleteDataDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
}) {
  const { t } = useTranslation();
  const [duration, setDuration] = useState("");
  const [unit, setUnit] = useState<Unit>("minutes");
  const [confirmText, setConfirmText] = useState("");
  const mutation = useDataDeleteRange();

  const parsedDuration = Number(duration);
  const validDuration =
    duration.trim() !== "" &&
    Number.isFinite(parsedDuration) &&
    parsedDuration > 0;
  const confirmOk = confirmText.trim().toLowerCase() === CONFIRM_PHRASE;
  const canSubmit = validDuration && confirmOk && !mutation.isPending;

  const reset = () => {
    setDuration("");
    setUnit("minutes");
    setConfirmText("");
  };

  const submit = () => {
    if (!canSubmit) return;
    const seconds = Math.floor(parsedDuration * UNIT_TO_SECONDS[unit]);
    mutation.mutate(seconds, {
      onSuccess: (result) => {
        toast.success(t.contextPopover.delete.toast(Number(result.framesDeleted)));
        reset();
        onOpenChange(false);
      },
      onError: (error) => {
        toast.error(t.contextPopover.delete.toastFailed(String(error)));
      },
    });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !mutation.isPending) reset();
        onOpenChange(next);
      }}
    >
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle className="font-display text-[17px] font-semibold tracking-[-0.015em]">
            {t.contextPopover.delete.confirm.title}
          </DialogTitle>
          <DialogDescription className="text-[13px] leading-[1.55]">
            {t.contextPopover.delete.confirm.description}
          </DialogDescription>
        </DialogHeader>

        <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-[12.5px] leading-[1.5] text-destructive">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{t.contextPopover.delete.confirm.warning}</span>
        </div>

        <div className="space-y-3 pt-1">
          <div>
            <Label className="mb-1.5 block text-[12px] font-medium tracking-[-0.005em] text-muted-foreground">
              {t.contextPopover.delete.confirm.durationLabel}
            </Label>
            <div className="flex gap-2">
              <Input
                value={duration}
                onChange={(event) => setDuration(event.target.value)}
                placeholder={t.contextPopover.delete.confirm.durationPlaceholder}
                inputMode="numeric"
                autoFocus
                disabled={mutation.isPending}
                className="flex-1"
              />
              <select
                value={unit}
                onChange={(event) => setUnit(event.target.value as Unit)}
                disabled={mutation.isPending}
                className="h-9 rounded-md border border-input bg-background px-2 text-[13px] focus:outline-none focus:ring-2 focus:ring-ring"
              >
                <option value="minutes">
                  {t.contextPopover.delete.confirm.unitMinutes}
                </option>
                <option value="hours">
                  {t.contextPopover.delete.confirm.unitHours}
                </option>
                <option value="days">
                  {t.contextPopover.delete.confirm.unitDays}
                </option>
              </select>
            </div>
          </div>

          <div>
            <Label className="mb-1.5 block text-[12px] font-medium tracking-[-0.005em] text-muted-foreground">
              {t.contextPopover.delete.confirm.confirmTypeLabel}
            </Label>
            <Input
              value={confirmText}
              onChange={(event) => setConfirmText(event.target.value)}
              placeholder={t.contextPopover.delete.confirm.confirmPlaceholder}
              disabled={mutation.isPending}
            />
          </div>
        </div>

        <DialogFooter className="gap-2 sm:gap-2">
          <Button
            type="button"
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={mutation.isPending}
          >
            {t.contextPopover.delete.confirm.cancel}
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={!canSubmit}
            onClick={submit}
          >
            {mutation.isPending
              ? t.contextPopover.delete.confirm.confirming
              : t.contextPopover.delete.confirm.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
