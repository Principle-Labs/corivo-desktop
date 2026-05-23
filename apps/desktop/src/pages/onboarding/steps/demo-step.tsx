import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  RefreshCw,
  Send,
  Square,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "@repo/ui/lib/utils";

import { useTranslation } from "@/i18n";
import { useChatStream, type LiveChatMessage } from "@/hooks/use-chat";
import { chatThreadCreate } from "@/lib/tauri";

const VISITED_DEMO_KEY = "corivo-onboarding-visited-demo";
const DOC_SEPARATOR = "\n\n---\nReference document:\n\n";

/**
 * Demo step — the user actually sends a real LLM request and the
 * Quick Ask popup expands to show a Quick-Ask-style conversation
 * (user bubble right / assistant text left). The PRD frame above
 * stays put — Corivo's "context" is always visible — so the layout
 * never swaps shapes underneath the user.
 *
 * Errors handled inline:
 *   - Empty input → Send disabled
 *   - Already streaming → Stop replaces Send
 *   - Thread create / send failure → ErrorBubble + Retry inline
 *   - Stream-mid network drop → assistant.error → ErrorBubble
 *   - User cancels → cancelled status, partial response retained
 *
 * The conversation persists to a real chat thread so it shows up in
 * /ask history as the user's first chat.
 */
export function DemoStep() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [threadId, setThreadId] = useState<string | null>(null);
  const [draft, setDraft] = useState(t.onboarding.demo.chip);

  const draftRef = useRef(draft);
  draftRef.current = draft;

  const stream = useChatStream(threadId, {
    createThreadIfMissing: async () => {
      const titleSeed = draftRef.current.trim().slice(0, 40);
      const thread = await chatThreadCreate(titleSeed || null);
      setThreadId(thread.id);
      return thread.id;
    },
    onThreadCreated: (id) => setThreadId(id),
  });

  useEffect(() => {
    try {
      sessionStorage.setItem(VISITED_DEMO_KEY, "1");
    } catch {
      // ignore: from-demo detection in permission-step degrades to
      // "user sees auto-advance again", not catastrophic.
    }
  }, []);

  const doc = t.onboarding.demo.doc;
  const docText = useMemo(() => serializeDoc(doc), [doc]);

  const handleSend = useCallback(() => {
    const text = draft.trim();
    if (!text) return;
    if (stream.isStreaming) return;
    const message = `${text}${DOC_SEPARATOR}${docText}`;
    void stream.sendMessage(message);
  }, [draft, docText, stream]);

  const handleStop = useCallback(() => {
    void stream.cancel();
  }, [stream]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (
        e.key === "Enter" &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.shiftKey &&
        !e.altKey
      ) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  // Auto-scroll the conversation to the bottom as new content streams
  // in. We pin to bottom only when the latest assistant is streaming;
  // once done the user can scroll freely without being yanked back.
  const conversationRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!stream.isStreaming) return;
    const el = conversationRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [stream.messages, stream.isStreaming]);

  const lastAssistant = useMemo<LiveChatMessage | null>(() => {
    const xs = stream.messages.filter((m) => m.role === "assistant");
    return xs.length > 0 ? xs[xs.length - 1] : null;
  }, [stream.messages]);

  const hasConversation =
    stream.messages.length > 0 || Boolean(stream.error);

  const handleBack = () => {
    void navigate({ to: "/onboarding/permission" });
  };

  const handleContinue = () => {
    void navigate({ to: "/onboarding/shortcut" });
  };

  const errorLabels = {
    retry: t.onboarding.demo.retry,
    errorTitle: t.onboarding.demo.errorLabel,
    networkHint: t.onboarding.demo.networkErrorHint,
  };

  return (
    <div className="relative flex h-full w-full flex-col items-center gap-5 py-5">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            "radial-gradient(circle at 50% 50%, color-mix(in oklab, var(--corivo-amber) 14%, transparent) 0%, transparent 65%)",
        }}
      />

      <div className="relative flex w-full max-w-[720px] flex-shrink-0 flex-col items-center gap-2.5 text-center">
        <h2 className="font-display text-[26px] font-semibold leading-[1.12] tracking-[-0.028em] text-foreground sm:text-[28px]">
          {t.onboarding.demo.title}
        </h2>
        <p className="text-[13.5px] leading-[1.55] text-muted-foreground sm:text-[14px]">
          {t.onboarding.demo.subtitle}
        </p>
      </div>

      {/* Theater: PRD frame (grows) + Quick Ask popup (natural / capped). */}
      <div className="relative flex w-full min-h-0 max-w-[720px] flex-1 flex-col">
        <div
          className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[14px] border bg-card"
          style={{
            borderColor: "var(--border-strong)",
            boxShadow: "var(--shadow-lg)",
          }}
        >
          <div
            className="flex h-[30px] flex-shrink-0 items-center gap-2.5 border-b border-border px-3"
            style={{ background: "var(--window-chrome)" }}
          >
            <div className="flex gap-[5px]">
              <span
                className="h-2 w-2 rounded-full"
                style={{ background: "#ec6a5e" }}
              />
              <span
                className="h-2 w-2 rounded-full"
                style={{ background: "#f5bf4f" }}
              />
              <span
                className="h-2 w-2 rounded-full"
                style={{ background: "#61c554" }}
              />
            </div>
            <span className="font-mono text-[10.5px] text-muted-foreground">
              {t.onboarding.demo.url}
            </span>
          </div>
          <div
            className="min-h-0 flex-1 overflow-y-auto px-8 pt-5 pb-6"
            style={{ scrollbarWidth: "thin" }}
          >
            <DocMock doc={doc} />
          </div>
        </div>

        {/* Quick Ask popup — title + (optional) conversation + input row.
            Visual focal point of the page, but the focus is achieved
            through ambient amber glow (large radius, low density —
            like warm light reflecting off a desk surface), not a
            hard amber border + solid ring. Border stays neutral so
            the popup reads as a precise object, not a banner. */}
        <div
          className="relative z-[2] mx-auto -mt-6 flex w-[82%] max-w-[560px] flex-shrink-0 flex-col gap-1.5 rounded-[12px] border bg-card px-2 pt-2 pb-2"
          style={{
            borderColor: "var(--border-strong)",
            boxShadow: [
              // base elevation (multi-layer, theme-aware)
              "var(--shadow-lg)",
              // close warm ambient — small offset, hints depth
              "0 6px 20px -8px color-mix(in oklab, var(--corivo-amber) 24%, transparent)",
              // far warm halo — large radius, very soft, draws the
              // eye without reading as a UI affordance
              "0 28px 80px -20px color-mix(in oklab, var(--corivo-amber) 36%, transparent)",
            ].join(", "),
            animation:
              "corivo-popup-pop 0.6s cubic-bezier(0.16, 1, 0.3, 1) both",
          }}
        >
          <span
            className="block px-1 text-center text-[12px] font-semibold tracking-[-0.005em]"
            style={{ color: "var(--accent-foreground)" }}
          >
            {t.onboarding.demo.askTitle}
          </span>

          {hasConversation ? (
            <div
              ref={conversationRef}
              className="flex flex-col gap-3 overflow-y-auto px-1 py-1"
              style={{
                maxHeight: "clamp(80px, 25vh, 220px)",
                scrollbarWidth: "thin",
              }}
            >
              {stream.messages.map((msg) =>
                renderMessage(msg, {
                  isLast: msg.id === lastAssistant?.id,
                  streamError: stream.error,
                  onRetry: handleSend,
                  labels: errorLabels,
                  thinkingLabel: t.onboarding.demo.thinking,
                }),
              )}
              {/* Edge case: stream.error fires before any assistant
                  message exists (e.g. thread create failed). */}
              {stream.error && !lastAssistant ? (
                <ErrorBubble
                  error={stream.error}
                  networkLikely={isNetworkLikely(stream.error)}
                  onRetry={handleSend}
                  labels={errorLabels}
                />
              ) : null}
            </div>
          ) : null}

          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <input
                type="text"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={handleKeyDown}
                disabled={stream.isStreaming}
                placeholder={t.onboarding.demo.askPlaceholder}
                className="h-10 w-full rounded-[8px] border border-border bg-background pl-3.5 pr-9 text-[14px] font-medium tracking-[-0.005em] text-foreground outline-none transition-colors placeholder:font-normal placeholder:text-muted-foreground focus:border-[var(--border-strong)] disabled:opacity-60"
              />
              {!stream.isStreaming ? (
                <kbd
                  aria-hidden
                  className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded-[4px] border border-[var(--border-strong)] bg-muted px-1.5 py-0.5 font-mono text-[10px] leading-none text-muted-foreground"
                >
                  ↩
                </kbd>
              ) : null}
            </div>
            {stream.isStreaming ? (
              <button
                type="button"
                onClick={handleStop}
                className="inline-flex h-10 items-center gap-1.5 rounded-[8px] border border-border bg-card px-3 text-[12.5px] font-medium text-foreground transition-colors hover:bg-muted"
                aria-label={t.onboarding.demo.stop}
              >
                <Square className="h-3 w-3 fill-current" />
                {t.onboarding.demo.stop}
              </button>
            ) : (
              <button
                type="button"
                onClick={handleSend}
                disabled={!draft.trim()}
                className={cn(
                  "inline-flex h-10 items-center gap-1.5 rounded-[8px] px-3 text-[12.5px] font-medium transition-colors",
                  "bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--corivo-amber-hover)]",
                  "disabled:cursor-default disabled:opacity-50",
                )}
                style={{ boxShadow: "var(--shadow-sm)" }}
                aria-label={t.onboarding.demo.send}
              >
                <Send className="h-3.5 w-3.5" />
                {t.onboarding.demo.send}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Footer CTAs — progressive importance:
          - Before any send: subdued "Skip" (same weight as ← back)
            keeps eyeballs on the popup, the actual focal point.
          - After the user has interacted (sent / errored): swaps to
            the amber primary "Continue" — visual cue that they've
            done the demo and the next step is now waiting. */}
      <div className="relative flex flex-shrink-0 items-center gap-5">
        <button
          type="button"
          onClick={handleBack}
          className="inline-flex items-center gap-1.5 text-[13px] text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          {t.onboarding.nav.back}
        </button>
        {hasConversation ? (
          <button
            type="button"
            onClick={handleContinue}
            className="inline-flex h-11 items-center justify-center gap-2 rounded-[10px] bg-[var(--primary)] px-6 text-[14px] font-medium tracking-[-0.005em] text-[var(--primary-foreground)] transition-colors hover:bg-[var(--corivo-amber-hover)]"
            style={{ boxShadow: "var(--shadow-sm)" }}
          >
            {t.onboarding.demo.cta}
            <ArrowRight className="h-4 w-4" />
          </button>
        ) : (
          <button
            type="button"
            onClick={handleContinue}
            className="text-[13px] text-muted-foreground transition-colors hover:text-foreground"
          >
            {t.onboarding.demo.skip}
          </button>
        )}
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// Conversation rendering — Quick-Ask-style bubbles (mirrors the
// `density="compact"` shape used in the real Quick Ask window):
//   - User: amber tint, right-aligned, max-w 85%, rounded-xl
//   - Assistant: full-width plain text, no surrounding bubble
//   - Errors: inline error card with retry button
// ─────────────────────────────────────────────────────────────────

interface ErrorLabels {
  retry: string;
  errorTitle: string;
  networkHint: string;
}

interface RenderContext {
  isLast: boolean;
  streamError: string | null;
  onRetry: () => void;
  labels: ErrorLabels;
  thinkingLabel: string;
}

function renderMessage(msg: LiveChatMessage, ctx: RenderContext) {
  if (msg.role === "user") {
    // The PRD blob we appended is hidden; only the original question
    // shows in the user bubble.
    const cleanContent = msg.content.split(DOC_SEPARATOR)[0] ?? msg.content;
    return <UserBubble key={msg.id} content={cleanContent} />;
  }

  // Assistant
  const inheritedError = msg.error || (ctx.isLast ? ctx.streamError : null);
  if (inheritedError) {
    return (
      <ErrorBubble
        key={msg.id}
        error={inheritedError}
        networkLikely={isNetworkLikely(inheritedError)}
        onRetry={ctx.onRetry}
        labels={ctx.labels}
      />
    );
  }

  if (msg.isStreaming && !msg.content) {
    return <ThinkingDots key={msg.id} label={ctx.thinkingLabel} />;
  }

  return <AssistantBubble key={msg.id} content={msg.content} />;
}

function UserBubble({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div
        className="max-w-[85%] whitespace-pre-wrap break-words rounded-xl px-3.5 py-2 text-[13px] leading-[1.55]"
        style={{
          background: "var(--accent)",
          color: "var(--accent-foreground)",
        }}
      >
        {content}
      </div>
    </div>
  );
}

/**
 * Assistant bubble — renders the LLM reply as markdown so **bold**,
 * lists, code, and headings show up properly. Mirrors the styling
 * approach of `AssistantMarkdown` in message-bubble.tsx but scoped
 * to the demo's compact 13px bubble.
 */
function AssistantBubble({ content }: { content: string }) {
  return (
    <div className="flex justify-start">
      <div
        className={cn(
          "max-w-[90%] break-words rounded-xl border border-border px-3.5 py-2 text-[13px] leading-[1.6] text-foreground",
          // markdown — paragraphs / lists / inline marks
          "[&_p]:my-1 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0",
          "[&_ul]:my-1 [&_ul]:list-disc [&_ul]:pl-5",
          "[&_ol]:my-1 [&_ol]:list-decimal [&_ol]:pl-5",
          "[&_li]:my-0.5",
          "[&_strong]:font-semibold",
          "[&_em]:italic",
          "[&_del]:line-through [&_del]:text-muted-foreground",
          "[&_a]:text-foreground [&_a]:underline [&_a]:underline-offset-2",
          // code — bg-card stands out against the bubble's bg-muted
          "[&_code]:rounded [&_code]:bg-card [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[11px]",
          "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border [&_pre]:bg-card [&_pre]:p-2.5",
          "[&_pre>code]:bg-transparent [&_pre>code]:p-0",
          // blockquote
          "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground",
          // headings — keep them small (we're inside a bubble)
          "[&_h1]:font-display [&_h1]:text-[14px] [&_h1]:font-semibold [&_h1]:mt-2 [&_h1:first-child]:mt-0",
          "[&_h2]:font-display [&_h2]:text-[13.5px] [&_h2]:font-semibold [&_h2]:mt-2 [&_h2:first-child]:mt-0",
          "[&_h3]:font-display [&_h3]:text-[13px] [&_h3]:font-semibold [&_h3]:mt-2 [&_h3:first-child]:mt-0",
          // GFM tables
          "[&_table]:my-2 [&_table]:block [&_table]:w-full [&_table]:overflow-x-auto",
          "[&_table]:border-collapse [&_table]:text-[11.5px]",
          "[&_th]:border [&_th]:border-border [&_th]:bg-card [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_th]:font-semibold",
          "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1 [&_td]:align-top",
          // GFM task lists
          "[&_li.task-list-item]:list-none [&_li.task-list-item]:-ml-5",
          "[&_li.task-list-item>input]:mr-1.5 [&_li.task-list-item>input]:align-middle",
        )}
        style={{
          background: "var(--muted)",
          boxShadow: "var(--shadow-sm)",
        }}
      >
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
      </div>
    </div>
  );
}

function ErrorBubble({
  error,
  networkLikely,
  onRetry,
  labels,
}: {
  error: string;
  networkLikely: boolean;
  onRetry: () => void;
  labels: ErrorLabels;
}) {
  return (
    <div
      className="flex flex-col gap-2 rounded-[8px] border px-3 py-2.5 text-[12.5px]"
      style={{
        borderColor: "color-mix(in oklab, var(--destructive) 30%, transparent)",
        background: "color-mix(in oklab, var(--destructive) 5%, var(--card))",
      }}
    >
      <div
        className="flex items-center gap-2 font-medium"
        style={{ color: "var(--destructive)" }}
      >
        <AlertTriangle className="h-3.5 w-3.5" />
        <span>{labels.errorTitle}</span>
      </div>
      <span className="text-[12px] leading-[1.55] text-muted-foreground">
        {networkLikely ? labels.networkHint : error}
      </span>
      <button
        type="button"
        onClick={onRetry}
        className="inline-flex w-fit items-center gap-1.5 rounded-[6px] border border-border bg-card px-2.5 py-1 text-[11.5px] font-medium text-foreground transition-colors hover:bg-muted"
      >
        <RefreshCw className="h-3 w-3" />
        {labels.retry}
      </button>
    </div>
  );
}

function ThinkingDots({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
      <span className="inline-flex items-center gap-1">
        <Dot delayMs={0} />
        <Dot delayMs={150} />
        <Dot delayMs={300} />
      </span>
      <span>{label}</span>
    </div>
  );
}

function Dot({ delayMs }: { delayMs: number }) {
  return (
    <span
      className="block h-1 w-1 rounded-full bg-current"
      style={{
        animation: "corivo-thinking-pulse 1.2s ease-in-out infinite",
        animationDelay: `${delayMs}ms`,
      }}
    />
  );
}

function isNetworkLikely(error: string): boolean {
  return /network|fetch|failed|disconnected|timeout|连接|网络/i.test(error);
}

// ─────────────────────────────────────────────────────────────────
// Document mock + serialization (used as both the visible PRD and
// the inline reference appended to the user's message).
// ─────────────────────────────────────────────────────────────────

interface DocShape {
  title: string;
  meta: string;
  bgH: string;
  bgP: string;
  goalH: string;
  goalP: string;
  planH: string;
  plan1: string;
  plan2: string;
  plan3: string;
  plan4: string;
  msH: string;
  ms1: string;
  ms2: string;
  ms3: string;
  ms4: string;
  riskH: string;
  riskP: string;
}

function serializeDoc(doc: DocShape): string {
  return [
    `# ${doc.title}`,
    doc.meta,
    "",
    `## ${doc.bgH}`,
    doc.bgP,
    "",
    `## ${doc.goalH}`,
    doc.goalP,
    "",
    `## ${doc.planH}`,
    `- ${doc.plan1}`,
    `- ${doc.plan2}`,
    `- ${doc.plan3}`,
    `- ${doc.plan4}`,
    "",
    `## ${doc.msH}`,
    `- ${doc.ms1}`,
    `- ${doc.ms2}`,
    `- ${doc.ms3}`,
    `- ${doc.ms4}`,
    "",
    `## ${doc.riskH}`,
    doc.riskP,
  ].join("\n");
}

function DocMock({ doc }: { doc: DocShape }) {
  return (
    <article className="flex flex-col text-foreground">
      <h1 className="font-display text-[22px] font-semibold leading-[1.2] tracking-[-0.018em]">
        {doc.title}
      </h1>
      <div className="mt-1 mb-4 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
        {doc.meta}
      </div>

      <DocSection h={doc.bgH}>
        <p className="text-[13px] leading-[1.65] text-foreground/75">
          {doc.bgP}
        </p>
      </DocSection>

      <DocSection h={doc.goalH}>
        <p className="text-[13px] leading-[1.65] text-foreground/75">
          {doc.goalP}
        </p>
      </DocSection>

      <DocSection h={doc.planH}>
        <ul className="list-disc pl-5 text-[13px] leading-[1.75] text-foreground/75 marker:text-foreground/40">
          <li>{doc.plan1}</li>
          <li>{doc.plan2}</li>
          <li>{doc.plan3}</li>
          <li>{doc.plan4}</li>
        </ul>
      </DocSection>

      <DocSection h={doc.msH}>
        <ul className="list-disc pl-5 text-[13px] leading-[1.75] text-foreground/75 marker:text-foreground/40">
          <li>{doc.ms1}</li>
          <li>{doc.ms2}</li>
          <li>{doc.ms3}</li>
          <li>{doc.ms4}</li>
        </ul>
      </DocSection>

      <DocSection h={doc.riskH} last>
        <p className="text-[13px] leading-[1.65] text-foreground/75">
          {doc.riskP}
        </p>
      </DocSection>
    </article>
  );
}

function DocSection({
  h,
  children,
  last,
}: {
  h: string;
  children: React.ReactNode;
  last?: boolean;
}) {
  return (
    <section className={last ? "mt-4" : "mb-3 mt-4 first:mt-0"}>
      <h2 className="mb-1.5 font-display text-[14px] font-semibold tracking-[-0.005em] text-foreground">
        {h}
      </h2>
      {children}
    </section>
  );
}
