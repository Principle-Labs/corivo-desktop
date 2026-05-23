import { useMemo } from "react";
import { ChevronRight, X } from "lucide-react";
import * as DialogPrimitive from "@radix-ui/react-dialog";

import { FocusMark } from "@repo/ui/components/focus-mark";
import { cn } from "@repo/ui/lib/utils";
import { useFrameDetail } from "@/hooks/use-frames";
import { useTranslation } from "@/i18n";
import type { Frame } from "@/lib/types";
import { useFrameDrawerStore } from "@/stores/frame-drawer-store";

/**
 * Right-side drawer that opens when a cited context chip is clicked
 * in /ask. Frames are an inline reference, not a destination.
 *
 * Built directly on Radix Dialog (rather than the shadcn Sheet
 * primitive) so the chrome can be exactly the design — minimal
 * padding, narrow header, custom close button.
 */
export function FrameDrawer() {
  const { t } = useTranslation();
  const frameId = useFrameDrawerStore((s) => s.frameId);
  const close = useFrameDrawerStore((s) => s.close);
  const detail = useFrameDetail(frameId);

  const data = detail.data ?? null;
  const open = frameId !== null;

  const appLabel =
    data?.app_name ?? data?.app_bundle_id ?? t.frame.unknownApp;
  const filename = data?.window_title ?? data?.url ?? appLabel;

  return (
    <DialogPrimitive.Root
      open={open}
      onOpenChange={(value) => {
        if (!value) close();
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          data-corivo-drawer-overlay=""
          className="fixed inset-0 z-50 bg-black/40"
        />
        <DialogPrimitive.Content
          data-corivo-drawer-content=""
          className={cn(
            "fixed inset-y-0 right-0 z-50 flex w-[440px] max-w-full flex-col bg-card",
            "border-l border-border shadow-[var(--shadow-lg)]",
            "will-change-transform",
          )}
        >
          <DialogPrimitive.Title className="sr-only">
            {filename}
          </DialogPrimitive.Title>
          <DialogPrimitive.Description className="sr-only">
            {t.frame.drawerEyebrow}
          </DialogPrimitive.Description>

          {/* Header — title left, close right. Sticks to the top so
           *  the close affordance never scrolls out of reach. */}
          <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border px-[18px] py-3">
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <FocusMark size={13} className="text-muted-foreground" />
              <span className="truncate text-[13px] font-semibold tracking-[-0.005em] text-foreground">
                {filename}
              </span>
            </div>
            <DialogPrimitive.Close
              className="flex h-[26px] w-[26px] items-center justify-center rounded-[4px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              aria-label="Close"
            >
              <X className="h-[14px] w-[14px]" />
            </DialogPrimitive.Close>
          </div>

          {detail.isLoading ? (
            <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">
              {t.frame.loading}
            </div>
          ) : !data ? (
            <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">
              {t.frame.notFound}
            </div>
          ) : (
            <>
              {/* Sticky meta row — time + app stay visible while the
               *  body scrolls so the user always knows what they're
               *  reading. */}
              <FrameMeta capturedAt={data.captured_at} appLabel={appLabel} />

              <div className="flex-1 overflow-y-auto">
                {/* Extracted text */}
                <section className="space-y-2.5 border-b border-border px-[18px] py-[16px]">
                  <SectionEyebrow>{t.frame.section.text}</SectionEyebrow>
                  <TextPanel
                    axText={data.ax_text}
                    ocrText={data.ocr_text}
                    emptyLabel={t.frame.emptyText}
                  />
                </section>

                {/* Raw payload — collapsed by default. Power users
                 *  who want the full record click to expand. */}
                <details className="group px-[18px] py-[14px]">
                  <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70 transition-colors hover:text-foreground [&::-webkit-details-marker]:hidden">
                    <ChevronRight className="h-3 w-3 transition-transform group-open:rotate-90" />
                    {t.frame.section.data}
                  </summary>
                  <div className="mt-2.5">
                    <RawPanel data={data} />
                  </div>
                </details>
              </div>
            </>
          )}
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function SectionEyebrow({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10.5px] font-medium uppercase tracking-[0.08em] text-muted-foreground/70">
      {children}
    </div>
  );
}

function FrameMeta({
  capturedAt,
  appLabel,
}: {
  capturedAt: string;
  appLabel: string;
}) {
  const time = useMemo(() => {
    const d = new Date(capturedAt);
    if (Number.isNaN(d.getTime())) return capturedAt;
    return d.toLocaleString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }, [capturedAt]);

  return (
    <div className="flex items-center gap-3 border-b border-border px-[18px] py-[10px] font-mono text-[10.5px] tracking-[0.06em] text-muted-foreground">
      <span>{time}</span>
      <span className="ml-auto truncate text-foreground/70">{appLabel}</span>
    </div>
  );
}

function TextPanel({
  axText,
  ocrText,
  emptyLabel,
}: {
  axText: string | null;
  ocrText: string | null;
  emptyLabel: string;
}) {
  const text = axText ?? ocrText;
  if (!text || text.trim() === "") {
    return <p className="text-[12px] text-muted-foreground">{emptyLabel}</p>;
  }
  return (
    <p className="whitespace-pre-wrap text-[12.5px] leading-[1.65] text-foreground/85">
      {text}
    </p>
  );
}

function RawPanel({ data }: { data: Frame }) {
  const view = useMemo(() => JSON.stringify(data, null, 2), [data]);
  return (
    <pre className="overflow-x-auto rounded-[6px] bg-muted px-3 py-3 font-mono text-[11px] leading-relaxed text-foreground/80">
      {view}
    </pre>
  );
}
