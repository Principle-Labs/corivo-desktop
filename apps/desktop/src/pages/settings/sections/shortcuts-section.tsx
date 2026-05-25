import { useQuery } from "@tanstack/react-query";

import { cn } from "@repo/ui/lib/utils";

import { useTranslation, type LocaleDict } from "@/i18n";
import type { HotkeyBinding } from "@/lib/types";
import { getHotkeyStatus } from "@/lib/tauri";

import { FieldGroup, SectionHeader } from "./settings-shared";

/**
 * Shortcuts — a single read-only surface that lists every keyboard
 * binding Corivo ships. Source of truth for what the user can press
 * anywhere in the app.
 *
 * Custom rebinding isn't supported in v1 (see Quick Ask hotkey notes);
 * this section is informational only. The global Quick Ask binding
 * shows live status via `hotkey_status` so a failed NSEvent-monitor
 * install is visible here rather than only inside the Quick Ask tab.
 */
type ShortcutRow = {
  key: keyof LocaleDict["settings"]["shortcuts"]["items"];
  combo: keyof LocaleDict["settings"]["shortcuts"]["keys"];
};

type ShortcutGroup = {
  scope: keyof LocaleDict["settings"]["shortcuts"]["scopes"];
  rows: ShortcutRow[];
};

const GROUPS: ShortcutGroup[] = [
  {
    scope: "global",
    rows: [
      { key: "quickAsk", combo: "doubleTapOption" },
      { key: "summonMain", combo: "cmdShiftO" },
    ],
  },
  {
    scope: "mainWindow",
    rows: [
      { key: "focusSearch", combo: "cmdK" },
      { key: "clearSearch", combo: "esc" },
    ],
  },
  {
    scope: "quickAsk",
    rows: [
      { key: "closePanel", combo: "escOrCmdW" },
      { key: "sendMessage", combo: "enter" },
      { key: "newline", combo: "shiftEnter" },
    ],
  },
  {
    scope: "ask",
    rows: [
      { key: "sendAsk", combo: "enterOrCmdEnter" },
      { key: "newline", combo: "shiftEnter" },
    ],
  },
];

const HOTKEY_KEY = ["hotkey-status"] as const;

export function ShortcutsSection() {
  const { t } = useTranslation();
  const status = useQuery({
    queryKey: HOTKEY_KEY,
    queryFn: getHotkeyStatus,
  });
  const installed = status.data?.installed;
  const installError = status.data?.error ?? null;

  return (
    <div className="max-w-2xl space-y-6">
      <SectionHeader
        title={t.settings.shortcuts.title}
        description={t.settings.shortcuts.description}
      />

      <StatusPill
        loading={status.isLoading}
        installed={installed}
        text={
          status.isLoading
            ? t.common.loading
            : installed === true
              ? t.settings.shortcuts.statusInstalled
              : installed === false
                ? t.settings.shortcuts.statusNotInstalled
                : t.settings.shortcuts.statusUnknown
        }
      />

      {installError ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-[11.5px] leading-[1.5] text-destructive">
          {installError}
        </div>
      ) : null}

      {GROUPS.map((group) => (
        <FieldGroup key={group.scope} title={t.settings.shortcuts.scopes[group.scope]}>
          <ul className="divide-y divide-border/60">
            {group.rows.map((row) => {
              const item = t.settings.shortcuts.items[row.key];
              const combo =
                row.key === "quickAsk"
                  ? quickAskCombo(status.data?.binding)
                  : row.combo;
              return (
                <li
                  key={`${group.scope}-${row.key}-${combo}`}
                  className="flex items-start justify-between gap-4 py-2.5 first:pt-0 last:pb-0"
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
                      {item.label}
                    </div>
                    <div className="mt-0.5 text-[11.5px] leading-[1.5] text-muted-foreground">
                      {item.description}
                    </div>
                  </div>
                  <KeyCombo label={t.settings.shortcuts.keys[combo]} />
                </li>
              );
            })}
          </ul>
        </FieldGroup>
      ))}

      <p className="text-[11px] leading-[1.55] text-muted-foreground">
        {t.settings.shortcuts.footnote}
      </p>
    </div>
  );
}

function quickAskCombo(
  binding: HotkeyBinding | undefined,
): keyof LocaleDict["settings"]["shortcuts"]["keys"] {
  if (binding === "double_tap_alt") {
    return "doubleTapAlt";
  }
  return "doubleTapOption";
}

function StatusPill({
  loading,
  installed,
  text,
}: {
  loading: boolean;
  installed: boolean | undefined;
  text: string;
}) {
  const tone =
    loading || installed === undefined
      ? "neutral"
      : installed
        ? "ok"
        : "warn";
  return (
    <div
      className={cn(
        "inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-[11.5px]",
        tone === "ok"
          ? "border-border bg-card text-foreground"
          : tone === "warn"
            ? "border-destructive/40 bg-destructive/5 text-destructive"
            : "border-border bg-muted/40 text-muted-foreground",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          tone === "ok"
            ? "bg-emerald-500"
            : tone === "warn"
              ? "bg-destructive"
              : "bg-muted-foreground/50",
        )}
      />
      <span>{text}</span>
    </div>
  );
}

function KeyCombo({ label }: { label: string }) {
  const parts = label.split(/\s+/).filter(Boolean);
  return (
    <span className="flex shrink-0 items-center gap-1">
      {parts.map((part, idx) => {
        if (part === "或" || part === "or" || part === "+") {
          return (
            <span
              key={`${part}-${idx}`}
              className="text-[10.5px] text-muted-foreground"
            >
              {part}
            </span>
          );
        }
        return <Kbd key={`${part}-${idx}`}>{part}</Kbd>;
      })}
    </span>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex h-[20px] min-w-[20px] items-center justify-center rounded-[4px] border border-border bg-card px-[6px] font-mono text-[11px] font-medium text-foreground shadow-[0_1px_0_rgba(28,26,23,0.04)]">
      {children}
    </kbd>
  );
}
