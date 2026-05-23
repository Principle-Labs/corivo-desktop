import { type ReactNode, useState } from "react";
import { ChevronLeft, ChevronRight, Settings2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@repo/ui/components/popover";

import {
  useCaptureStatus,
  useDataDeleteRange,
  usePauseCapture,
  useResumeCapture,
} from "@/hooks/use-capture";
import { useTranslation } from "@/i18n";
import { openSettingsDialog } from "@/stores/settings-dialog-store";

import { ContextAwarenessIntroCard } from "./intro-card";
import { DeleteDataDialog } from "./delete-data-dialog";

/**
 * The bottom-left "Context enabled · 上下文已启用" popover.
 *
 * Structurally inspired by Littlebird's Data-and-Privacy menu but
 * adapted to Corivo's existing primitives (Radix Popover + Dialog,
 * shadcn styling). The popover surfaces four things, in priority order:
 *
 *   1. A one-time intro card explaining what "context awareness" means.
 *      Sticky-dismissable so repeat opens stay quiet.
 *   2. Pause submenu — five durations from 5 min to 1 day, calling the
 *      existing `capture_pause` Tauri command.
 *   3. Delete submenu — last 5 / 15 min via `data_delete_range`, plus a
 *      custom dialog for arbitrary windows (with the `type 'delete' to
 *      confirm` gate borrowed from the hard-delete flow).
 *   4. A Manage row that jumps to the privacy settings section.
 *
 * The pause / delete submenus are rendered as the SAME popover swapping
 * views (`view: 'main' | 'pause' | 'delete'`) rather than nested
 * flyouts. Nested Radix popovers stack poorly on macOS — the inner one
 * loses focus restoration when the outer closes — and the view-swap
 * idiom is also closer to what mobile menus do, which is the model
 * users come in with.
 *
 * Trigger composition: the popover is invoked as a wrapper around
 * `StatusIndicator` via `PopoverTrigger asChild`. That keeps the
 * indicator's visual contract owned by `StatusIndicator` itself and
 * means this file is purely behavioral.
 */

type View = "main" | "pause" | "delete";

type PauseOption = {
  key: keyof typeof PAUSE_KEYS;
  seconds: number;
};

const PAUSE_KEYS = {
  fiveMin: 5 * 60,
  fifteenMin: 15 * 60,
  thirtyMin: 30 * 60,
  oneHour: 60 * 60,
  oneDay: 24 * 60 * 60,
} as const;

const PAUSE_OPTIONS: PauseOption[] = [
  { key: "fiveMin", seconds: PAUSE_KEYS.fiveMin },
  { key: "fifteenMin", seconds: PAUSE_KEYS.fifteenMin },
  { key: "thirtyMin", seconds: PAUSE_KEYS.thirtyMin },
  { key: "oneHour", seconds: PAUSE_KEYS.oneHour },
  { key: "oneDay", seconds: PAUSE_KEYS.oneDay },
];

const QUICK_DELETE_OPTIONS = [
  { key: "lastFiveMin" as const, seconds: 5 * 60 },
  { key: "lastFifteenMin" as const, seconds: 15 * 60 },
];

export function ContextPopover({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<View>("main");
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);

  const { data: status } = useCaptureStatus();
  const pauseMutation = usePauseCapture();
  const resumeMutation = useResumeCapture();
  const deleteMutation = useDataDeleteRange();

  const isPaused = !!status?.paused_until;
  const isRunning = status?.phase === "running";

  // Reset view to main whenever the popover closes — we don't want it
  // to remember a half-traversed submenu state from the last time.
  const handleOpenChange = (next: boolean) => {
    setOpen(next);
    if (!next) setView("main");
  };

  const handlePause = (option: PauseOption) => {
    const label = t.contextPopover.pause.options[option.key];
    pauseMutation.mutate(option.seconds, {
      onSuccess: () => {
        toast.success(t.contextPopover.pause.toast(label));
        setOpen(false);
        setView("main");
      },
      onError: (error) => {
        toast.error(t.contextPopover.pause.toastFailed(String(error)));
      },
    });
  };

  const handleResume = () => {
    resumeMutation.mutate(undefined, {
      onSuccess: () => {
        toast.success(t.contextPopover.resume.toast);
        setOpen(false);
      },
      onError: (error) => {
        toast.error(t.contextPopover.resume.toastFailed(String(error)));
      },
    });
  };

  const handleQuickDelete = (seconds: number) => {
    deleteMutation.mutate(seconds, {
      onSuccess: (result) => {
        toast.success(t.contextPopover.delete.toast(Number(result.framesDeleted)));
        setOpen(false);
        setView("main");
      },
      onError: (error) => {
        toast.error(t.contextPopover.delete.toastFailed(String(error)));
      },
    });
  };

  const handleManage = () => {
    setOpen(false);
    openSettingsDialog({ section: "capture", cardId: "exclusions" });
  };

  return (
    <>
      <Popover open={open} onOpenChange={handleOpenChange}>
        <PopoverTrigger asChild aria-label={t.contextPopover.triggerAriaLabel}>
          {children}
        </PopoverTrigger>
        <PopoverContent
          align="start"
          side="top"
          sideOffset={8}
          className="w-[300px] p-3"
        >
          {view === "main" && (
            <MainView
              isPaused={isPaused}
              isRunning={isRunning}
              onOpenPause={() => setView("pause")}
              onOpenDelete={() => setView("delete")}
              onResume={handleResume}
              onManage={handleManage}
              resuming={resumeMutation.isPending}
            />
          )}
          {view === "pause" && (
            <SubmenuView
              title={t.contextPopover.pause.label}
              onBack={() => setView("main")}
            >
              {PAUSE_OPTIONS.map((option) => (
                <MenuRow
                  key={option.key}
                  label={t.contextPopover.pause.options[option.key]}
                  onClick={() => handlePause(option)}
                  disabled={pauseMutation.isPending}
                />
              ))}
            </SubmenuView>
          )}
          {view === "delete" && (
            <SubmenuView
              title={t.contextPopover.delete.label}
              onBack={() => setView("main")}
            >
              {QUICK_DELETE_OPTIONS.map((option) => (
                <MenuRow
                  key={option.key}
                  label={t.contextPopover.delete.options[option.key]}
                  onClick={() => handleQuickDelete(option.seconds)}
                  disabled={deleteMutation.isPending}
                />
              ))}
              <MenuRow
                label={t.contextPopover.delete.options.custom}
                onClick={() => {
                  setOpen(false);
                  setDeleteDialogOpen(true);
                }}
                disabled={deleteMutation.isPending}
              />
            </SubmenuView>
          )}
        </PopoverContent>
      </Popover>

      <DeleteDataDialog
        open={deleteDialogOpen}
        onOpenChange={setDeleteDialogOpen}
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// Sub-components — kept inline because they're nowhere else used and the
// state lives on the parent.
// ---------------------------------------------------------------------------

function MainView({
  isPaused,
  isRunning,
  onOpenPause,
  onOpenDelete,
  onResume,
  onManage,
  resuming,
}: {
  isPaused: boolean;
  isRunning: boolean;
  onOpenPause: () => void;
  onOpenDelete: () => void;
  onResume: () => void;
  onManage: () => void;
  resuming: boolean;
}) {
  const { t } = useTranslation();
  const statusLabel = isPaused
    ? t.contextPopover.statusRow.paused
    : isRunning
      ? t.contextPopover.statusRow.running
      : t.contextPopover.statusRow.stopped;
  const dotClass = isRunning
    ? "bg-[var(--corivo-amber)]"
    : "bg-muted-foreground/40";

  return (
    <div className="space-y-2">
      <div className="px-1 text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/80">
        {t.contextPopover.header}
      </div>

      <ContextAwarenessIntroCard />

      <div className="space-y-0.5">
        {isPaused ? (
          <MenuRow
            label={t.contextPopover.resume.label}
            onClick={onResume}
            disabled={resuming}
          />
        ) : (
          <MenuRow
            label={t.contextPopover.pause.label}
            onClick={onOpenPause}
            trailing={<ChevronRight className="h-3.5 w-3.5" />}
          />
        )}
        <MenuRow
          label={t.contextPopover.delete.label}
          onClick={onOpenDelete}
          trailing={<ChevronRight className="h-3.5 w-3.5" />}
        />
      </div>

      <div className="border-t border-border" />

      <div className="flex items-center justify-between gap-2 px-1 py-1">
        <span className="text-[12px] text-foreground/80">
          {t.contextPopover.exclusions.label}
        </span>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          onClick={onManage}
          className="h-7 gap-1 px-2 text-[11.5px]"
        >
          <Settings2 className="h-3 w-3" />
          {t.contextPopover.exclusions.manage}
        </Button>
      </div>

      <div className="border-t border-border" />

      <div className="flex items-center gap-2 px-1 py-1 text-[12px] text-foreground/80">
        <span className={`h-1.5 w-1.5 rounded-full ${dotClass}`} />
        {statusLabel}
      </div>
    </div>
  );
}

function SubmenuView({
  title,
  onBack,
  children,
}: {
  title: string;
  onBack: () => void;
  children: ReactNode;
}) {
  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={onBack}
        className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      >
        <ChevronLeft className="h-3 w-3" />
        {title}
      </button>
      <div className="space-y-0.5">{children}</div>
    </div>
  );
}

function MenuRow({
  label,
  onClick,
  disabled,
  trailing,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  trailing?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] text-foreground/90 transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span>{label}</span>
      {trailing ? (
        <span className="text-muted-foreground">{trailing}</span>
      ) : null}
    </button>
  );
}
