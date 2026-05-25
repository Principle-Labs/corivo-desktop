import type { Trigger, Weekday } from "@corivo/shared-types";

import type { useTranslation } from "@/i18n";

type T = ReturnType<typeof useTranslation>["t"];

/**
 * Compact UTC-instant → local-time formatter for list rows. Drops the
 * year when it matches today's year so the row stays narrow.
 */
export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const now = new Date();
  const sameYear = date.getFullYear() === now.getFullYear();
  const opts: Intl.DateTimeFormatOptions = {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  };
  if (!sameYear) {
    opts.year = "numeric";
  }
  return new Intl.DateTimeFormat(undefined, opts).format(date);
}

const WEEKDAY_LABELS: Record<Weekday, string> = {
  mon: "一",
  tue: "二",
  wed: "三",
  thu: "四",
  fri: "五",
  sat: "六",
  sun: "日",
};

/**
 * Human-readable single-line description of a trigger. Used both in
 * the list row and the create/edit drawer summary.
 */
export function describeTrigger(trigger: Trigger, t: T): string {
  switch (trigger.kind) {
    case "interval":
      return `${t.workflows.drawer.trigger.interval} · ${trigger.minutes} ${t.workflows.drawer.trigger.minutes}`;
    case "daily":
      return `${t.workflows.drawer.trigger.daily} ${pad(trigger.hour)}:${pad(trigger.minute)} (${trigger.tz})`;
    case "weekly": {
      const days = trigger.weekdays.map((w) => WEEKDAY_LABELS[w]).join("/");
      return `${t.workflows.drawer.trigger.weekly} 周${days} ${pad(trigger.hour)}:${pad(trigger.minute)} (${trigger.tz})`;
    }
    case "once":
      return `${t.workflows.drawer.trigger.once} · ${formatDateTime(trigger.at)}`;
    case "cron":
      return `Cron \`${trigger.expr}\` (${trigger.tz})`;
  }
}

function pad(n: number): string {
  return n.toString().padStart(2, "0");
}

/**
 * Best-effort guess at the OS's IANA timezone. Falls back to
 * `Asia/Shanghai` when the runtime doesn't expose one (very rare).
 */
export function guessLocalTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Shanghai";
  } catch {
    return "Asia/Shanghai";
  }
}
