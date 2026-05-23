import { useMemo, useState, type AnchorHTMLAttributes } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

import { FocusMark } from "@repo/ui/components/focus-mark";
import { cn } from "@repo/ui/lib/utils";
import type { ModelDirectory } from "@corivo/shared-types";

import type {
  AssistantSegment,
  LiveChatMessage,
  LiveToolEvent,
} from "@/hooks/use-chat";
import { Mascot } from "@/components/brand/mascot";
import { useTranslation, type LocaleDict } from "@/i18n";
import { modelsGetAvailable } from "@/lib/tauri";
import { useFrameDrawerStore } from "@/stores/frame-drawer-store";

interface MessageBubbleProps {
  message: LiveChatMessage;
  /** Optional density hint. Quick Ask uses "compact" to tighten margins
   *  inside the small floating panel; `/ask` uses "comfortable". */
  density?: "comfortable" | "compact";
}

/**
 * Renders a single chat message — both roles use a full-width "turn"
 * shape (head row + content body), per screens-v0 §02. No surrounding
 * bubble: bubbles tighten the layout but make the conversation read
 * as IM rather than a thoughtful exchange. Quick Ask still uses the
 * older compact bubble shape — pass `density="compact"` for that.
 */
export function MessageBubble({
  message,
  density = "comfortable",
}: MessageBubbleProps) {
  if (density === "compact") return <CompactBubble message={message} />;

  if (message.role === "user") return <UserTurn message={message} />;
  return <AssistantTurn message={message} />;
}

// ─────────────────────────────────────────────────────────────────
// User turn (full-width, screens-v0 §02)
// ─────────────────────────────────────────────────────────────────

function UserTurn({ message }: { message: LiveChatMessage }) {
  return (
    <article className="flex flex-col items-end gap-1.5">
      <div
        className={cn(
          "max-w-[80%] rounded-[14px] px-4 py-2.5 text-[14.5px] leading-[1.55] text-foreground",
          "border border-border bg-card shadow-[0_1px_2px_rgba(28,26,23,0.025)]",
        )}
      >
        <p className="whitespace-pre-wrap">{message.content}</p>
      </div>
      <CitedFrameChips
        frameIds={message.citedFrameIds}
        userFrameLabel={message.userFrameLabel ?? null}
        align="end"
      />
    </article>
  );
}

// ─────────────────────────────────────────────────────────────────
// Assistant turn (full-width, head row with FocusMark + active tool pill)
// ─────────────────────────────────────────────────────────────────

function AssistantTurn({ message }: { message: LiveChatMessage }) {
  const { t } = useTranslation();
  const isEmpty = message.segments.length === 0;
  const lastIdx = message.segments.length - 1;
  const fold = !message.isStreaming && splitFinalAnswer(message.segments);
  const activeTool = useMemo(
    () => firstActiveTool(message.segments),
    [message.segments],
  );

  // Copy text rolls up every visible text/thinking segment, in order.
  // Tool calls and frame citations don't go on the clipboard — that
  // would surface raw tool ids and JSON which is noise for the user.
  const copyText = useMemo(() => {
    const buf: string[] = [];
    for (const seg of message.segments) {
      if (seg.kind === "text" || seg.kind === "thinking") {
        if (seg.text) buf.push(seg.text);
      }
    }
    return buf.join("\n\n").trim();
  }, [message.segments]);

  return (
    <article className="group flex flex-col gap-2">
      <header className="flex items-center gap-2 text-[11.5px] tracking-[0.04em] text-muted-foreground">
        <Mascot size="xs" />
        <span className="font-sans text-[12.5px] font-semibold tracking-[-0.005em] text-foreground">
          {t.chat.corivoLabel}
        </span>
        {message.isStreaming ? (
          <span className="font-mono">· {t.chat.streaming}</span>
        ) : message.modelUsed ? (
          <ModelAttribution alias={message.modelUsed} />
        ) : null}
        {activeTool ? (
          <span className="ml-auto">
            <ToolCallPill name={activeTool} />
          </span>
        ) : null}
      </header>

      <div className="text-[14.5px] leading-[1.65] text-foreground">
        {fold ? (
          <>
            <ProcessFold segments={fold.process} />
            <TextSegment segment={fold.answer} showCursor={false} />
          </>
        ) : (
          message.segments.map((segment, idx) => {
            const isLast = idx === lastIdx;
            if (segment.kind === "text") {
              return (
                <TextSegment
                  key={idx}
                  segment={segment}
                  showCursor={isLast && message.isStreaming}
                />
              );
            }
            if (segment.kind === "thinking") {
              return (
                <ThinkingSegment
                  key={idx}
                  segment={segment}
                  showCursor={isLast && message.isStreaming}
                />
              );
            }
            return <ToolCluster key={idx} events={segment.events} />;
          })
        )}
        {isEmpty && message.isStreaming && (
          <ThinkingDots status={message.status} />
        )}
        {message.isStreaming &&
          !isEmpty &&
          lastEndsWithTools(message.segments) && (
            <ThinkingDots status={message.status} />
          )}
        {!message.isStreaming && isEmpty && !message.error && (
          <span className="text-muted-foreground">{t.chat.emptyReply}</span>
        )}
        {message.error && (
          <p className="mt-2 text-[12px] text-destructive">{message.error}</p>
        )}
      </div>

      {/* Footer row: copy + cited chips. Copy lives at the end of the
       *  message instead of next to the "Corivo" header — easier to
       *  read against the actual content the user wants to grab. */}
      {(copyText.length > 0 && !message.isStreaming) ||
      message.citedFrameIds.length > 0 ? (
        <div className="flex items-center gap-2">
          {!message.isStreaming && copyText.length > 0 ? (
            <CopyButton text={copyText} />
          ) : null}
          <CitedFrameChips frameIds={message.citedFrameIds} />
        </div>
      ) : null}
    </article>
  );
}

/**
 * Compact "via <model>" label in the assistant header. Reads the
 * directory cache (in-memory, fed by `models_get_available`) to
 * resolve an alias to its display_name. Falls back to the raw alias
 * when the directory hasn't loaded yet or the alias has been revoked
 * since the turn ran — never errors, never crashes a bubble over a
 * stale audit row.
 */
function ModelAttribution({ alias }: { alias: string }) {
  const { data: directory } = useQuery<ModelDirectory>({
    queryKey: ["models-available"],
    queryFn: modelsGetAvailable,
    refetchOnWindowFocus: false,
  });
  const entry = directory?.managed.find((m) => m.alias === alias);
  const label = entry?.display_name ?? alias;
  return (
    <span
      className="font-mono text-[11px] tracking-[0.04em] text-muted-foreground"
      title={alias}
    >
      · {label}
    </span>
  );
}

// ─────────────────────────────────────────────────────────────────
// Cited frame chips (used by both user + assistant turns)
// ─────────────────────────────────────────────────────────────────

function CitedFrameChips({
  frameIds,
  userFrameLabel,
  align = "start",
}: {
  frameIds: readonly string[];
  /** Quick Ask sends carry a precomputed label like "VSCode · file.tsx".
   *  When present we render a single labeled pill (the user's focus
   *  anchor); other citations stay numbered. */
  userFrameLabel?: string | null;
  /** L/R alignment — user bubbles want chips right-aligned to mirror
   *  the bubble itself; assistant turns want left-aligned. */
  align?: "start" | "end";
}) {
  const { t } = useTranslation();
  const open = useFrameDrawerStore((s) => s.open);

  if (!frameIds || frameIds.length === 0) return null;

  // User turns with an explicit focus anchor render the original
  // labeled pill — that label carries information ("VSCode · file.tsx")
  // so we keep the wide pill shape.
  if (userFrameLabel && frameIds.length === 1) {
    return (
      <div
        className={cn(
          "flex",
          align === "end" ? "justify-end" : "justify-start",
        )}
      >
        <button
          type="button"
          onClick={() => open(frameIds[0])}
          className="inline-flex items-center gap-2 rounded-full border border-border bg-[var(--muted)] px-3 py-1 text-[11.5px] tracking-[-0.005em] text-muted-foreground transition-colors hover:border-[color-mix(in_oklab,var(--foreground)_13%,transparent)] hover:text-foreground"
        >
          <FocusMark size={11} className="opacity-70" />
          <span className="font-medium">{userFrameLabel}</span>
        </button>
      </div>
    );
  }

  // Multiple citations (typical assistant turn): footnote-style.
  // Tiny "引用" label + a row of small numbered chips. Each chip is
  // a unique anchor so the user can pattern-match by index instead of
  // staring at ten copies of the same generic label.
  return (
    <div
      className={cn(
        "flex flex-wrap items-center gap-1.5",
        align === "end" ? "justify-end" : "justify-start",
      )}
    >
      <span className="text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70">
        {t.chat.citationsLabel}
      </span>
      {frameIds.map((frameId, idx) => (
        <button
          key={frameId}
          type="button"
          onClick={() => open(frameId)}
          aria-label={t.chat.citationAria(idx + 1)}
          title={t.chat.citationAria(idx + 1)}
          className="inline-flex h-[20px] min-w-[20px] items-center justify-center rounded-[5px] border border-border bg-[var(--muted)] px-[5px] text-[10.5px] font-medium tabular-nums text-muted-foreground transition-colors hover:border-[color-mix(in_oklab,var(--foreground)_18%,transparent)] hover:text-foreground"
        >
          {idx + 1}
        </button>
      ))}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// Copy button (hover-revealed on assistant turns).
// Lives in `chat/` so it works in both /ask and the Quick Ask
// transcript — same MessageBubble component renders both.
// ─────────────────────────────────────────────────────────────────

function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch (e) {
      console.warn("clipboard write failed", e);
    }
  };
  return (
    <button
      type="button"
      onClick={onCopy}
      title={copied ? t.chat.copied : t.chat.copy}
      aria-label={copied ? t.chat.copied : t.chat.copy}
      className={cn(
        "inline-flex h-6 w-6 items-center justify-center rounded-[5px] text-muted-foreground transition-all",
        "hover:bg-muted hover:text-foreground",
        // On the assistant turn the button stays hidden until the
        // article is hovered. Once the user clicks copy, we keep it
        // visible briefly so the success checkmark is readable.
        copied
          ? "opacity-100"
          : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
      )}
    >
      {copied ? (
        <Check className="h-[13px] w-[13px]" strokeWidth={2.2} />
      ) : (
        <Copy className="h-[13px] w-[13px]" />
      )}
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────
// Tool call pill (amber, screens-v0 §02 ".tool-call")
// ─────────────────────────────────────────────────────────────────

function ToolCallPill({ name }: { name: string }) {
  return (
    <span
      className="inline-flex items-center gap-2 rounded-full border border-transparent px-3 py-1 font-sans text-[11.5px] font-medium tracking-[-0.005em]"
      style={{
        backgroundColor: "var(--accent)",
        color: "var(--accent-foreground)",
      }}
    >
      <span
        className="h-[5px] w-[5px] rounded-full"
        style={{
          backgroundColor: "var(--corivo-amber)",
          animation: "corivo-status-pulse 1.4s ease-in-out infinite",
        }}
      />
      {name}
    </span>
  );
}

// ─────────────────────────────────────────────────────────────────
// Compact bubble (Quick Ask path — kept for back-compat)
// ─────────────────────────────────────────────────────────────────

function CompactBubble({ message }: { message: LiveChatMessage }) {
  const { t } = useTranslation();
  if (message.role === "user") {
    const frameHint = userFrameHint(message, t);
    return (
      <div className="ml-auto flex max-w-[85%] flex-col items-end gap-1">
        <div className="whitespace-pre-wrap break-words rounded-xl bg-accent px-3 py-1.5 text-sm text-accent-foreground">
          {message.content}
        </div>
        {frameHint ? (
          <span className="text-[10px] text-muted-foreground">
            {frameHint}
          </span>
        ) : null}
      </div>
    );
  }
  // Assistant compact: simpler timeline without head row, plus a
  // tiny hover-revealed copy button at the top-right so Quick Ask
  // gets the same affordance as /ask.
  const isEmpty = message.segments.length === 0;
  const lastIdx = message.segments.length - 1;
  const fold = !message.isStreaming && splitFinalAnswer(message.segments);
  const copyText = message.segments
    .filter(
      (s): s is { kind: "text" | "thinking"; text: string } =>
        s.kind === "text" || s.kind === "thinking",
    )
    .map((s) => s.text)
    .filter(Boolean)
    .join("\n\n")
    .trim();
  return (
    <div className="group space-y-1.5 text-sm leading-relaxed text-foreground">
      {fold ? (
        <>
          <ProcessFold segments={fold.process} />
          <TextSegment segment={fold.answer} showCursor={false} />
        </>
      ) : (
        message.segments.map((segment, idx) => {
          const isLast = idx === lastIdx;
          if (segment.kind === "text") {
            return (
              <TextSegment
                key={idx}
                segment={segment}
                showCursor={isLast && message.isStreaming}
              />
            );
          }
          if (segment.kind === "thinking") {
            return (
              <ThinkingSegment
                key={idx}
                segment={segment}
                showCursor={isLast && message.isStreaming}
              />
            );
          }
          return <ToolCluster key={idx} events={segment.events} />;
        })
      )}
      {isEmpty && message.isStreaming && (
        <ThinkingDots status={message.status} />
      )}
      {!message.isStreaming && isEmpty && !message.error && (
        <span className="text-muted-foreground">{t.chat.emptyReply}</span>
      )}
      {message.error && (
        <p className="text-xs text-destructive">{message.error}</p>
      )}
      {!message.isStreaming && copyText.length > 0 ? (
        <div className="pt-0.5">
          <CopyButton text={copyText} />
        </div>
      ) : null}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

// Avatar / initialOf removed alongside the user-turn header row —
// user messages are now bubble-only so the head label was redundant.

function firstActiveTool(segments: AssistantSegment[]): string | null {
  for (let i = segments.length - 1; i >= 0; i -= 1) {
    const seg = segments[i];
    if (seg && seg.kind === "tools") {
      const pending = seg.events.find((e) => e.result === null);
      if (pending) return shortToolName(pending.name);
    }
  }
  return null;
}

function shortToolName(name: string): string {
  return name.replace(/^mcp__[^_]+__/, "");
}

type FinalAnswerSplit = {
  process: AssistantSegment[];
  answer: Extract<AssistantSegment, { kind: "text" }>;
};

function splitFinalAnswer(
  segments: AssistantSegment[],
): FinalAnswerSplit | null {
  if (segments.length < 2) return null;
  const last = segments[segments.length - 1];
  if (last.kind !== "text") return null;
  const process = segments.slice(0, -1);
  if (process.length === 0) return null;
  return { process, answer: last };
}

function userFrameHint(
  message: LiveChatMessage,
  t: LocaleDict,
): string | null {
  if (message.userFrameLabel) return message.userFrameLabel;
  const ids = message.citedFrameIds;
  if (ids.length === 0) return null;
  return ids.length === 1
    ? t.chat.frameAnchorOne
    : t.chat.frameAnchorN(ids.length);
}

function lastEndsWithTools(segments: AssistantSegment[]): boolean {
  const last = segments[segments.length - 1];
  if (!last || last.kind !== "tools") return false;
  return last.events.every((e) => e.result !== null);
}

function TextSegment({
  segment,
  showCursor,
}: {
  segment: { kind: "text"; text: string };
  showCursor: boolean;
}) {
  return (
    <div className="relative">
      <AssistantMarkdown content={segment.text} />
      {showCursor && <BlinkingCursor />}
    </div>
  );
}

function ThinkingSegment({
  segment,
  showCursor,
}: {
  segment: { kind: "thinking"; text: string };
  showCursor: boolean;
}) {
  return (
    <div className="relative border-l-2 border-border/60 pl-3 italic text-muted-foreground">
      <span className="mr-1 not-italic" aria-hidden>
        ✦
      </span>
      <span className="whitespace-pre-wrap">{segment.text}</span>
      {showCursor && <BlinkingCursor />}
    </div>
  );
}

function ThinkingDots({ status }: { status?: string }) {
  const { t } = useTranslation();
  return (
    <div
      className="flex items-center gap-2 py-1 text-muted-foreground"
      aria-label={status ?? t.chat.thinking}
    >
      <div className="flex items-center gap-1">
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current [animation-delay:-0.3s]" />
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current [animation-delay:-0.15s]" />
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current" />
      </div>
      {status && <span className="text-xs">{status}</span>}
    </div>
  );
}

/** Subtle breathing cursor at the tail of a streaming text segment. */
function BlinkingCursor() {
  return (
    <span
      aria-hidden
      className="ml-0.5 inline-block h-3.5 w-[2px] animate-cursor-breathe bg-[var(--corivo-amber)] align-middle"
      style={{ borderRadius: "1px" }}
    />
  );
}

/**
 * Render the assistant's reply as markdown. Claude routinely emits
 * **bold**, bullets, code spans, and links — surfacing them as raw
 * source text was the bug we kept seeing.
 *
 * GFM is enabled (tables, strikethrough, task lists, autolinks) via
 * `remark-gfm`. Styling is inline with arbitrary-attribute selectors
 * so we don't need to pull in `@tailwindcss/typography` for one
 * component.
 */
function AssistantMarkdown({ content }: { content: string }) {
  return (
    <div
      className={[
        "space-y-2 text-[14.5px] leading-[1.65]",
        "[&_p]:my-1.5 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0",
        "[&_ul]:my-1.5 [&_ul]:list-disc [&_ul]:pl-5",
        "[&_ol]:my-1.5 [&_ol]:list-decimal [&_ol]:pl-5",
        "[&_li]:my-0.5",
        "[&_strong]:font-semibold",
        "[&_em]:italic",
        "[&_del]:line-through [&_del]:text-muted-foreground",
        "[&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5",
        "[&_code]:font-mono [&_code]:text-[12px]",
        "[&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border [&_pre]:bg-muted [&_pre]:p-3",
        "[&_pre>code]:bg-transparent [&_pre>code]:p-0",
        "[&_a]:text-foreground [&_a]:underline [&_a]:underline-offset-2",
        "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3",
        "[&_blockquote]:text-muted-foreground",
        "[&_h1]:font-display [&_h1]:text-base [&_h1]:font-semibold [&_h1]:mt-2",
        "[&_h2]:font-display [&_h2]:text-base [&_h2]:font-semibold [&_h2]:mt-2",
        "[&_h3]:font-display [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:mt-2",
        // GFM tables — wrap in horizontal scroll on overflow.
        "[&_table]:my-2 [&_table]:block [&_table]:w-full [&_table]:overflow-x-auto",
        "[&_table]:border-collapse [&_table]:text-xs",
        "[&_th]:border [&_th]:border-border [&_th]:bg-muted/40 [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_th]:font-semibold",
        "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1 [&_td]:align-top",
        // GFM task lists — strip the bullet, align the checkbox inline.
        "[&_li.task-list-item]:list-none [&_li.task-list-item]:-ml-5",
        "[&_li.task-list-item>input]:mr-1.5 [&_li.task-list-item>input]:align-middle",
      ].join(" ")}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{ a: ExternalLink }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

/**
 * Anchor renderer for assistant markdown. The Tauri WebView would
 * otherwise navigate the chat surface itself when a link is clicked —
 * route to the system browser via the opener plugin instead.
 *
 * Only http/https/mailto are forwarded; anything else (relative
 * fragments, javascript:, file://, …) is dropped silently so a
 * malformed link can't replace the WebView.
 */
function ExternalLink({
  href,
  children,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement>) {
  return (
    <a
      {...rest}
      href={href}
      onClick={(event) => {
        event.preventDefault();
        if (!href) return;
        if (/^(https?:|mailto:)/i.test(href)) {
          void openUrl(href);
        }
      }}
    >
      {children}
    </a>
  );
}

/**
 * Wraps the pre-answer timeline (intermediate text + tool clusters) in
 * a single collapsed "思考过程" disclosure. Shown once the turn has
 * settled — while streaming, segments render inline so the user can
 * watch the model think. Expanded view re-uses the same `TextSegment`
 * + `ToolCluster` renderers, so the interleaved order is preserved.
 */
function ProcessFold({ segments }: { segments: AssistantSegment[] }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        type="button"
        className="flex items-center gap-1 text-left text-xs text-muted-foreground transition hover:text-foreground"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
      >
        {open ? (
          <ChevronDown className="h-3 w-3" />
        ) : (
          <ChevronRight className="h-3 w-3" />
        )}
        <span>{t.chat.thoughtProcess}</span>
      </button>
      {open && (
        <div className="ml-4 mt-2 space-y-2 border-l border-border/60 pl-3">
          {segments.map((segment, idx) => {
            if (segment.kind === "text") {
              return (
                <TextSegment key={idx} segment={segment} showCursor={false} />
              );
            }
            if (segment.kind === "thinking") {
              return (
                <ThinkingSegment
                  key={idx}
                  segment={segment}
                  showCursor={false}
                />
              );
            }
            return <ToolCluster key={idx} events={segment.events} />;
          })}
        </div>
      )}
    </div>
  );
}

function ToolCluster({ events }: { events: LiveToolEvent[] }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const hasPending = events.some((e) => e.result === null);

  return (
    <div className="text-xs text-muted-foreground">
      <button
        type="button"
        className="flex items-center gap-1 text-left transition hover:text-foreground"
        onClick={() => setOpen((o) => !o)}
      >
        {open ? (
          <ChevronDown className="h-3 w-3" />
        ) : (
          <ChevronRight className="h-3 w-3" />
        )}
        <span>
          {describeCluster(events, t)}
          {hasPending ? ` · ${t.chat.inProgress}` : ""}
        </span>
      </button>
      {open && (
        <ul className="ml-4 mt-1.5 space-y-1.5">
          {events.map((event) => (
            <li
              key={event.id}
              className="rounded-md border border-border/60 bg-muted/40 p-2"
            >
              <div className="font-mono text-[11px] text-foreground/80">
                {event.name}({JSON.stringify(event.args)})
              </div>
              {event.result !== null && (
                <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words text-[10px] text-muted-foreground">
                  {typeof event.result === "string"
                    ? event.result
                    : JSON.stringify(event.result, null, 2)}
                </pre>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function describeCluster(events: LiveToolEvent[], t: LocaleDict): string {
  const counts = new Map<string, number>();
  const order: string[] = [];
  for (const e of events) {
    const verb = verbForTool(e.name, t);
    if (!counts.has(verb)) {
      order.push(verb);
      counts.set(verb, 0);
    }
    counts.set(verb, (counts.get(verb) ?? 0) + 1);
  }
  return order
    .map((verb) => {
      const n = counts.get(verb) ?? 0;
      return n > 1 ? `${verb}${t.chat.toolVerbs.timesSuffix(n)}` : verb;
    })
    .join(", ");
}

function toolVerbsTable(t: LocaleDict): Record<string, string> {
  const v = t.chat.toolVerbs;
  return {
    mcp__corivo__recall_screen_history: v.recallFrames,
    recall_screen_history: v.recallFrames,
    Bash: v.runCommand,
    Read: v.readFile,
    Edit: v.editFile,
    Write: v.writeFile,
    Grep: v.grep,
    Glob: v.glob,
    WebFetch: v.webFetch,
    WebSearch: v.webSearch,
    TodoWrite: v.todo,
  };
}

function verbForTool(name: string, t: LocaleDict): string {
  const table = toolVerbsTable(t);
  if (table[name]) return table[name];
  const bare = name.replace(/^mcp__[^_]+__/, "");
  return t.chat.toolVerbs.genericCall(bare);
}
