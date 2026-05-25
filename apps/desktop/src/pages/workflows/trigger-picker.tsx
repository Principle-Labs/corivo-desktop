import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Trigger, TriggerKind, Weekday } from "@corivo/shared-types";

import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";

import { useTranslation } from "@/i18n";
import { formatDateTime, guessLocalTimezone } from "@/pages/workflows/format";
import { workflowsPreviewTrigger } from "@/lib/tauri";

const ALL_WEEKDAYS: Weekday[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const KINDS: TriggerKind[] = ["interval", "daily", "weekly", "once", "cron"];

interface Props {
  trigger: Trigger;
  onChange: (next: Trigger) => void;
}

/**
 * Trigger picker — kind tabs + per-kind form + a live "next firing"
 * preview that round-trips through the Rust validator. Validation +
 * next-after computation live in the backend so the drawer never has
 * to ship its own copy of the calendar math.
 */
export function TriggerPicker({ trigger, onChange }: Props) {
  const { t } = useTranslation();

  const { data: preview } = useQuery({
    queryKey: ["trigger-preview", trigger],
    queryFn: () => workflowsPreviewTrigger(trigger),
    staleTime: 1000,
  });

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-1">
        {KINDS.map((kind) => (
          <button
            key={kind}
            type="button"
            onClick={() => onChange(defaultsForKind(kind))}
            className={`rounded-md border px-2.5 py-1 text-[11.5px] transition-colors ${
              trigger.kind === kind
                ? "border-foreground/40 bg-foreground/5 text-foreground"
                : "border-border/40 text-muted-foreground hover:border-border/70"
            }`}
          >
            {labelForKind(kind, t)}
          </button>
        ))}
      </div>

      <TriggerForm trigger={trigger} onChange={onChange} />

      <PreviewLine preview={preview} t={t} />
    </div>
  );
}

function TriggerForm({ trigger, onChange }: Props) {
  const { t } = useTranslation();
  switch (trigger.kind) {
    case "interval":
      return (
        <Field label={t.workflows.drawer.trigger.minutes}>
          <Input
            type="number"
            min={1}
            value={trigger.minutes}
            onChange={(e) =>
              onChange({
                ...trigger,
                minutes: Math.max(1, parseInt(e.target.value || "0", 10) || 1),
              })
            }
          />
        </Field>
      );
    case "daily":
      return (
        <div className="grid grid-cols-3 gap-2">
          <HourField value={trigger.hour} onChange={(hour) => onChange({ ...trigger, hour })} />
          <MinuteField value={trigger.minute} onChange={(minute) => onChange({ ...trigger, minute })} />
          <TzField value={trigger.tz} onChange={(tz) => onChange({ ...trigger, tz })} />
        </div>
      );
    case "weekly":
      return (
        <div className="flex flex-col gap-2">
          <Field label={t.workflows.drawer.trigger.weekdays}>
            <div className="flex flex-wrap gap-1">
              {ALL_WEEKDAYS.map((w) => {
                const selected = trigger.weekdays.includes(w);
                return (
                  <button
                    key={w}
                    type="button"
                    onClick={() => {
                      const next = selected
                        ? trigger.weekdays.filter((x) => x !== w)
                        : [...trigger.weekdays, w];
                      onChange({ ...trigger, weekdays: next });
                    }}
                    className={`rounded-md border px-2 py-1 text-[11.5px] ${
                      selected
                        ? "border-foreground/40 bg-foreground/5 text-foreground"
                        : "border-border/40 text-muted-foreground"
                    }`}
                  >
                    周{weekdayShort(w)}
                  </button>
                );
              })}
            </div>
          </Field>
          <div className="grid grid-cols-3 gap-2">
            <HourField value={trigger.hour} onChange={(hour) => onChange({ ...trigger, hour })} />
            <MinuteField value={trigger.minute} onChange={(minute) => onChange({ ...trigger, minute })} />
            <TzField value={trigger.tz} onChange={(tz) => onChange({ ...trigger, tz })} />
          </div>
        </div>
      );
    case "once": {
      // <input type="datetime-local"> wants `YYYY-MM-DDTHH:mm` in
      // local time. The store is UTC, so we round-trip via Date.
      const localValue = toLocalInput(trigger.at);
      return (
        <Field label={t.workflows.drawer.trigger.at}>
          <Input
            type="datetime-local"
            value={localValue}
            onChange={(e) => {
              const parsed = fromLocalInput(e.target.value);
              if (parsed) onChange({ ...trigger, at: parsed });
            }}
          />
        </Field>
      );
    }
    case "cron":
      return (
        <div className="flex flex-col gap-2">
          <Field
            label={t.workflows.drawer.trigger.cronExpr}
            hint={t.workflows.drawer.trigger.cronHint}
          >
            <Input
              value={trigger.expr}
              placeholder="0 9 * * *"
              onChange={(e) => onChange({ ...trigger, expr: e.target.value })}
              className="font-mono"
            />
          </Field>
          <TzField value={trigger.tz} onChange={(tz) => onChange({ ...trigger, tz })} />
        </div>
      );
  }
}

function PreviewLine({
  preview,
  t,
}: {
  preview:
    | { valid: boolean; next_run_at: string | null; error: string | null }
    | undefined;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  if (!preview) return null;
  if (!preview.valid) {
    return (
      <p className="text-[11.5px] text-destructive">
        {preview.error ?? t.workflows.drawer.trigger.previewInvalid}
      </p>
    );
  }
  if (!preview.next_run_at) {
    return (
      <p className="text-[11.5px] text-muted-foreground">
        {t.workflows.drawer.trigger.previewNone}
      </p>
    );
  }
  return (
    <p className="text-[11.5px] text-muted-foreground">
      {t.workflows.drawer.trigger.preview(formatDateTime(preview.next_run_at))}
    </p>
  );
}

function HourField({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  const { t } = useTranslation();
  return (
    <Field label={t.workflows.drawer.trigger.hour}>
      <Input
        type="number"
        min={0}
        max={23}
        value={value}
        onChange={(e) => onChange(clamp(parseInt(e.target.value || "0", 10) || 0, 0, 23))}
      />
    </Field>
  );
}

function MinuteField({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  const { t } = useTranslation();
  return (
    <Field label={t.workflows.drawer.trigger.minute}>
      <Input
        type="number"
        min={0}
        max={59}
        value={value}
        onChange={(e) => onChange(clamp(parseInt(e.target.value || "0", 10) || 0, 0, 59))}
      />
    </Field>
  );
}

function TzField({ value, onChange }: { value: string; onChange: (s: string) => void }) {
  const { t } = useTranslation();
  // OS tz is plenty for v1; the user can hand-edit to any IANA name
  // if they want something else (the backend validates).
  const [local] = useState(() => guessLocalTimezone());
  useEffect(() => {
    if (!value) onChange(local);
    // Run-once on mount to default an empty value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Field label={t.workflows.drawer.trigger.tz}>
      <Input value={value} onChange={(e) => onChange(e.target.value)} />
    </Field>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Label className="text-[11.5px] text-muted-foreground">{label}</Label>
      {children}
      {hint ? (
        <p className="text-[10.5px] text-muted-foreground/70">{hint}</p>
      ) : null}
    </div>
  );
}

function labelForKind(kind: TriggerKind, t: ReturnType<typeof useTranslation>["t"]): string {
  switch (kind) {
    case "interval":
      return t.workflows.drawer.trigger.interval;
    case "daily":
      return t.workflows.drawer.trigger.daily;
    case "weekly":
      return t.workflows.drawer.trigger.weekly;
    case "once":
      return t.workflows.drawer.trigger.once;
    case "cron":
      return t.workflows.drawer.trigger.cron;
  }
}

function weekdayShort(w: Weekday): string {
  return ({ mon: "一", tue: "二", wed: "三", thu: "四", fri: "五", sat: "六", sun: "日" })[w];
}

function clamp(n: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, n));
}

/**
 * Produce a default trigger for the picked kind. Keeps the kind tabs
 * idempotent: switching from `daily` to `weekly` and back lands you
 * back on the same starting point (today 09:00 local).
 */
export function defaultsForKind(kind: TriggerKind): Trigger {
  const tz = guessLocalTimezone();
  switch (kind) {
    case "interval":
      return { kind: "interval", minutes: 60 };
    case "daily":
      return { kind: "daily", hour: 9, minute: 0, tz };
    case "weekly":
      return { kind: "weekly", weekdays: ["mon"], hour: 9, minute: 0, tz };
    case "once": {
      // Default to "tomorrow at this minute" so the field reads as a
      // future commitment rather than an instantly-fired event.
      const tomorrow = new Date();
      tomorrow.setDate(tomorrow.getDate() + 1);
      return { kind: "once", at: tomorrow.toISOString() };
    }
    case "cron":
      return { kind: "cron", expr: "0 9 * * *", tz };
  }
}

function toLocalInput(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fromLocalInput(value: string): string | null {
  if (!value) return null;
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return null;
  return d.toISOString();
}
