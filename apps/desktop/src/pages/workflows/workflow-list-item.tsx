import type { WorkflowView } from "@corivo/shared-types";
import { Loader2 } from "lucide-react";

import { Button } from "@repo/ui/components/button";
import { Switch } from "@repo/ui/components/switch";

import { useTranslation } from "@/i18n";
import { describeTrigger, formatDateTime } from "@/pages/workflows/format";
import { useWorkflowInFlightStore } from "@/stores/workflow-in-flight-store";

interface Props {
  view: WorkflowView;
  onToggle: (enabled: boolean) => void;
  onRunNow: () => void;
  onCancel: () => void;
  onEdit: () => void;
  onShowHistory: () => void;
  onDelete: () => void;
}

/**
 * One row in the workflows list. Renders identity + trigger summary
 * + last-run status + the row's action cluster. Stateless on its own;
 * the parent (`WorkflowsPage`) holds the mutations and re-fetches.
 */
export function WorkflowListItem({
  view,
  onToggle,
  onRunNow,
  onCancel,
  onEdit,
  onShowHistory,
  onDelete,
}: Props) {
  const { t } = useTranslation();
  const { definition, schedule } = view;
  const enabled = schedule?.enabled ?? false;
  const inFlight = useWorkflowInFlightStore((s) =>
    s.entries.has(definition.slug),
  );
  const triggerLabel = schedule ? describeTrigger(schedule.trigger, t) : null;
  // While a run is in flight, hide both `lastRunLabel` and
  // `nextRunLabel`. They describe historical state — "尚未运行" /
  // "上次失败 · 2 分钟前" — and reading "正在运行… · 尚未运行" on
  // the same line reads as a contradiction. The amber chip is the
  // only state the user needs right now; the schedule metadata
  // comes back the moment the run finishes.
  const lastRunLabel = inFlight ? null : formatLastRun(view, t);
  const nextRunLabel =
    !inFlight && schedule?.next_run_at
      ? t.workflows.list.nextRun(formatDateTime(schedule.next_run_at))
      : null;

  return (
    <li className="flex flex-col gap-2 rounded-lg border border-border/40 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-display text-[15px] font-medium tracking-[-0.005em] text-foreground">
              {definition.name}
            </span>
            <code className="rounded bg-muted px-1.5 py-[1px] font-mono text-[10.5px] text-muted-foreground">
              {definition.slug}
            </code>
            {schedule?.source === "agent" ? (
              <span
                title={t.workflows.list.agentBadgeTitle}
                className="rounded-[4px] bg-foreground/8 px-1.5 py-[1px] text-[10.5px] font-medium text-foreground/80"
              >
                {t.workflows.list.agentBadge}
              </span>
            ) : null}
          </div>
          {definition.description ? (
            <p className="line-clamp-2 max-w-2xl text-[12.5px] text-muted-foreground">
              {definition.description}
            </p>
          ) : null}
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-muted-foreground">
            {inFlight ? (
              <span className="inline-flex items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-[11px] font-medium text-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t.workflows.list.running}
              </span>
            ) : null}
            {triggerLabel ? <span>{triggerLabel}</span> : null}
            {nextRunLabel ? <span>· {nextRunLabel}</span> : null}
            {lastRunLabel ? <span>· {lastRunLabel}</span> : null}
          </div>
        </div>

        {/*
         * Right slot is ALWAYS a real <Switch>. The old design had two
         * different things in the same position — a pill-shaped toggle
         * for scheduled rows and plain "未设置时间" text for unscheduled
         * rows — which made the affordance unreadable: users couldn't
         * tell which one was clickable. Now the toggle shape is the
         * single, consistent signal; unscheduled rows just disable it
         * and surface a "设置时间" inline action that opens the editor.
         */}
        <div className="flex shrink-0 items-center gap-2">
          {schedule ? (
            <span className="text-[11px] text-muted-foreground">
              {enabled
                ? t.workflows.list.enabledOn
                : t.workflows.list.enabledOff}
            </span>
          ) : (
            <button
              type="button"
              onClick={onEdit}
              className="text-[11px] text-muted-foreground/80 underline-offset-4 transition-colors hover:text-foreground hover:underline"
            >
              {t.workflows.list.scheduleAction}
            </button>
          )}
          <Switch
            checked={enabled}
            onCheckedChange={schedule ? onToggle : undefined}
            disabled={!schedule}
            aria-label={
              schedule
                ? enabled
                  ? t.workflows.list.enabledOn
                  : t.workflows.list.enabledOff
                : t.workflows.list.unscheduledHint
            }
            title={
              schedule ? undefined : t.workflows.list.unscheduledHint
            }
          />
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1.5 pt-1">
        {inFlight ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={onCancel}
            // Slightly destructive coloring — Cancel is a stop
            // action; match the same red ghost used for 删除 so the
            // user reads it as "interrupt this" not "secondary
            // alternative to Run Now".
            className="border-destructive/40 text-destructive hover:bg-destructive/10 hover:text-destructive"
          >
            {t.workflows.list.cancel}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={onRunNow}
          >
            {t.workflows.list.runNow}
          </Button>
        )}
        <Button type="button" size="sm" variant="ghost" onClick={onEdit}>
          {t.workflows.list.edit}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={onShowHistory}
        >
          {t.workflows.list.history}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="text-destructive hover:text-destructive"
          onClick={onDelete}
        >
          {t.workflows.list.delete}
        </Button>
      </div>
    </li>
  );
}

function formatLastRun(
  view: WorkflowView,
  t: ReturnType<typeof useTranslation>["t"],
): string | null {
  const schedule = view.schedule;
  // No schedule → "last run" doesn't apply yet. Returning null collapses
  // the metadata line entirely instead of rendering an orphan "· 尚未运行"
  // (the leading "·" is a separator for the trigger summary, which is
  // also empty in this state). The right-side disabled Switch + "设置时间"
  // action already tells the user what's missing.
  if (!schedule) return null;
  if (!schedule.last_run_at) {
    return t.workflows.list.neverRun;
  }
  const when = formatDateTime(schedule.last_run_at);
  if (schedule.last_status === "failure") {
    return t.workflows.list.lastFailure(when);
  }
  return t.workflows.list.lastSuccess(when);
}

