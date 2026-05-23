import { useEffect, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, Square } from "lucide-react";

import { cn } from "@repo/ui/lib/utils";
import type { ModelDirectory } from "@corivo/shared-types";
import { Mascot } from "@/components/brand/mascot";
import { MessageBubble } from "@/components/chat/message-bubble";
import { useTranslation } from "@/i18n";
import type { LiveChatMessage } from "@/hooks/use-chat";
import { modelsGetAvailable, modelsSetActiveModel } from "@/lib/tauri";
import { useActiveThreadStore } from "@/stores/active-thread-store";

interface MessageStreamProps {
  messages: LiveChatMessage[];
  isStreaming: boolean;
  error: string | null;
  /** Picker is stateful: the active model lives on
   *  `accounts.activeAlias` server-side and on `directory.active_alias`
   *  in the cache. `onSend` is unaware of model choice — the backend
   *  reads the active alias at send time. */
  onSend: (content: string) => Promise<void>;
  onCancel?: () => Promise<void>;
  /** Identity used to key the composer draft (CO-63). Pass the real
   *  thread id when one is selected, or `DRAFT_THREAD_ID` while the
   *  user is in the type-to-create slot. Each value gets its own
   *  persistent composer text so switching sessions doesn't leak the
   *  in-progress draft across threads. */
  inputKey: string;
}

/**
 * Conversation column for the redesigned /ask (screens-v0 §02).
 *
 * Three-row vertical layout: scrollable conversation body (auto-stick
 * to bottom), divider, and a shadow-sm input "card" with text + send
 * affordance + footer hints. The card is the only emphasized surface
 * in the column — everything else is structural lines.
 */
export function MessageStream({
  messages,
  isStreaming,
  error,
  onSend,
  onCancel,
  inputKey,
}: MessageStreamProps) {
  const { t } = useTranslation();
  const input = useActiveThreadStore((s) => s.inputDrafts[inputKey] ?? "");
  const setInputDraft = useActiveThreadStore((s) => s.setInputDraft);
  const setInput = (value: string) => setInputDraft(inputKey, value);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Model picker — STATEFUL. Source of truth is the backend's
  // `accounts.activeAlias`, exposed via the directory's `active_alias`.
  // Clicking an alias dispatches `modelsSetActiveModel` which blocks
  // until the gateway-side group flip lands; React Query refreshes
  // the cached directory afterwards. While the mutation is pending
  // we show a spinner on the picker — sub2api PUT is fast (<300ms)
  // but the loading state makes the cause-and-effect obvious.
  const qc = useQueryClient();
  const { data: directory } = useQuery<ModelDirectory>({
    queryKey: ["models-available"],
    queryFn: modelsGetAvailable,
    refetchOnWindowFocus: false,
  });
  const switchModel = useMutation({
    mutationFn: modelsSetActiveModel,
    onSuccess: (next) => {
      qc.setQueryData<ModelDirectory>(["models-available"], next);
    },
  });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [messages]);

  // Sync textarea height with the per-thread draft value after a
  // thread switch — without this, switching from a thread with a
  // long draft to one with a short draft (or vice versa) would leave
  // the auto-grown height from the previous thread until the user
  // typed again. onChange handles the typing path; this handles the
  // external-value-change path.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [inputKey, input]);

  const submit = async () => {
    const content = input.trim();
    if (!content || isStreaming) return;
    setInput("");
    // Reset the composer to its single-row height immediately. The
    // `[inputKey, input]` effect above resyncs height on thread switch
    // but doesn't help here: by the time it runs, `isStreaming` has
    // flipped on and the textarea is `disabled`, so the scrollHeight
    // read still reports the multi-line size from before the clear
    // and the box stays pinned at that height until the next keystroke
    // (CO-59).
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
    await onSend(content);
  };

  const trimmedLength = input.trim().length;

  return (
    // h-full instead of flex-1 — AskPage no longer wraps us in a flex
    // column (the conversation header was removed in the redesign), so
    // we self-size to the layout shell directly. Inner body still
    // owns the scroll via `flex-1` + `min-h-0`.
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto px-8 py-7"
      >
        {messages.length === 0 ? (
          <EmptyState />
        ) : (
          <div className="mx-auto flex max-w-3xl flex-col gap-7">
            {messages.map((m) => (
              <MessageBubble key={m.id} message={m} />
            ))}
            {error ? (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
                {error}
              </div>
            ) : null}
          </div>
        )}
      </div>

      <div className="border-t border-border bg-background px-8 pb-5 pt-4">
        <form
          className="mx-auto max-w-3xl"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <div
            className={cn(
              "flex items-center gap-2.5 rounded-[10px] border px-3.5 py-2 transition-[box-shadow,border-color,background-color] duration-150",
              // Resting shadow: a single 1px drop with very low alpha —
              // shadow-sm felt too heavy in-context. Focus-within still
              // promotes to shadow-md so the active state stays clear,
              // but only when the composer is actually accepting input.
              isStreaming
                ? "border-dashed border-border/70 bg-muted/40 shadow-none"
                : [
                    "border-border bg-card shadow-[0_1px_2px_rgba(28,26,23,0.025)]",
                    "focus-within:border-[color-mix(in_oklab,var(--foreground)_22%,transparent)] focus-within:shadow-[var(--shadow-md)]",
                  ],
            )}
          >
            <textarea
              ref={textareaRef}
              data-composer-input
              className={cn(
                "block max-h-[180px] w-full flex-1 resize-none border-0 bg-transparent py-1 text-[14.5px] leading-[1.5] text-foreground placeholder:text-muted-foreground",
                // Suppress the global *:focus-visible 2px ring on the
                // textarea itself — the parent card already shows focus
                // via `focus-within:border-...`. !important is needed
                // because the global rule has equal specificity.
                "outline-none focus:outline-none focus-visible:outline-none focus-visible:!outline-0 focus-visible:!ring-0",
                "disabled:cursor-not-allowed disabled:opacity-60",
              )}
              rows={1}
              placeholder={
                isStreaming
                  ? t.ask.composer.streamingPlaceholder
                  : t.ask.composer.placeholder
              }
              value={input}
              onChange={(e) => {
                setInput(e.target.value);
                const el = e.currentTarget;
                el.style.height = "auto";
                el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
              }}
              onKeyDown={(e) => {
                // While an IME is composing (拼音/かな/한글 picking a
                // candidate), Enter means "confirm the candidate", not
                // "send the message". `keyCode === 229` is the legacy
                // signal for the Enter that closes composition — some
                // WebKit builds flip `isComposing` back to false on
                // that final keydown, so we need both.
                if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void submit();
                  return;
                }
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void submit();
                }
              }}
              disabled={isStreaming}
              aria-disabled={isStreaming}
            />
            {isStreaming && onCancel ? (
              <button
                type="button"
                title={t.ask.composer.stopHint}
                onClick={() => void onCancel()}
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] border border-[color-mix(in_oklab,var(--foreground)_18%,transparent)] bg-transparent text-foreground transition-colors hover:bg-muted"
              >
                <Square className="h-3 w-3 fill-current" />
              </button>
            ) : (
              <button
                type="submit"
                disabled={trimmedLength === 0}
                className={cn(
                  "flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition-colors",
                  trimmedLength > 0
                    ? "bg-foreground text-background hover:bg-foreground/90"
                    : "bg-muted text-muted-foreground",
                )}
              >
                <Check className="h-[13px] w-[13px]" strokeWidth={2.2} />
              </button>
            )}
          </div>
          <div className="mt-2 flex items-center justify-between font-mono text-[10.5px] tracking-[0.04em] text-muted-foreground">
            <span className="flex items-center gap-3">
              <span>{t.ask.composer.contextHint}</span>
              <ModelPicker
                directory={directory}
                onSwitch={(alias) => switchModel.mutate(alias)}
                pending={switchModel.isPending}
                disabled={isStreaming || switchModel.isPending}
              />
            </span>
            <span className="flex items-center gap-2">
              <span className="flex items-center gap-[2px]">
                <Kbd>⌘</Kbd>
                <Kbd>↵</Kbd>
              </span>
              <span>{t.ask.composer.send}</span>
              <span className="opacity-50">·</span>
              <span className="flex items-center gap-[2px]">
                <Kbd>⌘</Kbd>
                <Kbd>.</Kbd>
              </span>
              <span>{t.ask.composer.stop}</span>
            </span>
          </div>
        </form>
      </div>
    </div>
  );
}

/**
 * Inline composer picker. Stateful: dispatches `models_set_active_model`
 * which blocks until the gateway-side sub2api group flip lands. Shows
 * a spinner while the mutation is pending; the `<select>` reflects
 * the directory's `active_alias` once the cache refreshes.
 *
 * Hidden when the user has 0 or 1 aliases (nothing to switch between).
 * A bare `<select>` is fine here — the popover-style would dominate
 * the delicate footer row.
 */
function ModelPicker({
  directory,
  onSwitch,
  pending,
  disabled,
}: {
  directory: ModelDirectory | undefined;
  onSwitch: (alias: string) => void;
  pending: boolean;
  disabled: boolean;
}) {
  const managed = directory?.managed ?? [];
  if (managed.length < 2) return null;
  const currentAlias =
    directory?.active_alias ?? directory?.default_alias ?? managed[0].alias;
  const current =
    managed.find((m) => m.alias === currentAlias) ?? managed[0];
  return (
    <span className="inline-flex items-center gap-1.5">
      <select
        value={current.alias}
        onChange={(e) => {
          if (e.target.value !== current.alias) onSwitch(e.target.value);
        }}
        disabled={disabled}
        title={`当前模型：${current.display_name}`}
        className={cn(
          "rounded border border-border/40 bg-transparent px-1.5 py-[1px]",
          "font-mono text-[10.5px] tracking-[0.04em] text-muted-foreground",
          "hover:border-border focus:outline-none focus-visible:outline-none focus-visible:!ring-0",
          "disabled:cursor-not-allowed disabled:opacity-50",
        )}
      >
        {managed.map((m) => (
          <option key={m.alias} value={m.alias}>
            {m.display_name}
          </option>
        ))}
      </select>
      {pending ? (
        <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
      ) : null}
    </span>
  );
}

function EmptyState() {
  const { t } = useTranslation();
  return (
    <div className="mx-auto flex h-full max-w-md flex-col items-center justify-center gap-5 py-16 text-center">
      <Mascot size="md" breath />
      <div className="space-y-1.5">
        <p className="font-display text-[22px] font-semibold tracking-[-0.015em] text-foreground">
          {t.ask.empty.title}
        </p>
        <p className="text-[13px] text-muted-foreground">
          {t.ask.empty.typeHint}
        </p>
      </div>

      {/* Quick Ask gesture nudge — visual primary affordance.
       *  Empty state is the highest-leverage moment to teach
       *  double-tap ⌥ — the user has nothing to do here yet, so a
       *  gentle "try this" reads as helpful instead of nagging. */}
      <div className="mt-1 flex items-center gap-2 rounded-full border border-border bg-card/60 px-3.5 py-2 text-[12px] text-muted-foreground">
        <span className="flex items-center gap-[3px]">
          <Kbd>⌥</Kbd>
          <Kbd>⌥</Kbd>
        </span>
        <span>{t.ask.empty.hotkeyHint}</span>
      </div>
    </div>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[4px] bg-[color-mix(in_oklab,var(--foreground)_8%,transparent)] px-[5px] font-mono text-[10px] text-muted-foreground">
      {children}
    </span>
  );
}
