import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  Clock,
  ExternalLink,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";
import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@repo/ui/components/tabs";
import { useTranslation, type LocaleDict } from "@/i18n";
import {
  checkAxPermission,
  checkScreenRecordingPermission,
  exclusionAdd,
  exclusionList,
  exclusionRemove,
  framesList,
  fromInvokeError,
  getCaptureStatus,
  getSystemInfo,
  openAxSettings,
  openSystemSettingsPrivacy,
  restartAsAdministrator,
  startCapture,
  stopCapture,
} from "@/lib/tauri";
import {
  clearSettingsRequest,
  useSettingsRequestedCardId,
} from "@/stores/settings-dialog-store";

import { PrivacyFilterSection } from "./privacy-filter-section";
import { FieldGroup, SectionHeader, SettingsSkeleton } from "./settings-shared";

const EXCLUSION_KEY = ["exclusion-list"] as const;
const RECENT_FRAMES_KEY = ["recent-frames-24h"] as const;

/** Scroll target ids. The privacy popover's "Manage" button uses
 *  "exclusions" to drop the user straight onto the exclusions card. */
export const EXCLUSIONS_CARD_ID = "exclusions";

/**
 * Capture & Privacy — second tab in Settings (★ elevated in the
 * post-redesign IA, see docs/design/ia-v0.html). Holds:
 *
 *   1. Capture switch — start/stop
 *   2. Recent 24h inspector — frame count, size, peak hour, 24-bar histogram
 *   3. Quick Ask hotkey display
 *   4. Exclusion list
 *   5. PII filter — local-redaction model toggle + categories + maintenance
 *   6. System permissions (screen recording + AX)
 *
 * Replaces the old Permissions tab — they were two halves of the same
 * "what Corivo sees" surface and were artificially split.
 */
export function CapturePrivacySection() {
  const { t } = useTranslation();
  const requestedCardId = useSettingsRequestedCardId();
  const exclusionsRef = useRef<HTMLDivElement>(null);

  // Privacy popover → "Manage" button sets requestedCardId = "exclusions".
  // We scroll the card into view once it's mounted, then clear the
  // request so a subsequent manual open doesn't re-jump.
  //
  // The timeout exists because Radix's dialog has an open transition;
  // scrolling inside a still-animating container is what causes the
  // "snapped to top half-way then jumped" feel. 200 ms is a comfortable
  // upper bound for the dialog's default fade-in (~150 ms) and leaves
  // headroom on slower machines / reduced-motion disabled. A pure
  // animation-end listener would be more principled but Radix doesn't
  // expose one cleanly; the constant works in practice.
  useEffect(() => {
    if (requestedCardId !== EXCLUSIONS_CARD_ID) return;
    const id = window.setTimeout(() => {
      exclusionsRef.current?.scrollIntoView({
        block: "start",
        behavior: "smooth",
      });
      clearSettingsRequest("card");
    }, 200);
    return () => window.clearTimeout(id);
  }, [requestedCardId]);

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.capture.title}
        description={t.settings.capture.description}
      />

      <CaptureToggleCard />
      <RecentFramesInspector />
      <div id={EXCLUSIONS_CARD_ID} ref={exclusionsRef} className="scroll-mt-4">
        <ExclusionTabsCard />
      </div>
      {/* PII filter 渲染为 3 个 FieldGroup(启用 / 类目 / 维护),
          和上面的 cards 共用同一条 hairline 节奏。详见
          privacy-filter-section.tsx 的注释。 */}
      <PrivacyFilterSection />
      <PermissionsCard />
    </div>
  );
}

function CaptureToggleCard() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: ["capture-status"],
    queryFn: getCaptureStatus,
    refetchInterval: 3000,
  });
  const startMutation = useMutation({
    mutationFn: startCapture,
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["capture-status"] }),
    onError: (error) => toast.error(t.common.setupFailed(fromInvokeError(error))),
  });
  const stopMutation = useMutation({
    mutationFn: stopCapture,
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["capture-status"] }),
    onError: (error) => toast.error(t.common.setupFailed(fromInvokeError(error))),
  });

  const phase = status.data?.phase ?? "stopped";
  const isRunning = phase === "running";
  const pending = startMutation.isPending || stopMutation.isPending;

  return (
    <FieldGroup title={t.settings.capture.captureGroup}>
      <div className="flex items-start justify-between gap-4 rounded-md border border-border bg-card px-4 py-3.5">
        <div className="flex items-start gap-3">
          <span
            className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${
              isRunning
                ? "bg-[var(--corivo-amber)]"
                : "bg-muted-foreground/40"
            }`}
          />
          <div>
            <Label className="text-sm">
              {isRunning
                ? t.settings.capture.captureCard.runningTitle
                : t.settings.capture.captureCard.stoppedTitle}
            </Label>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {isRunning
                ? t.settings.capture.captureCard.runningDesc
                : t.settings.capture.captureCard.stoppedDesc}
            </p>
          </div>
        </div>
        <Button
          size="sm"
          variant={isRunning ? "outline" : "default"}
          disabled={pending}
          onClick={() => {
            if (isRunning) stopMutation.mutate();
            else startMutation.mutate();
          }}
        >
          {pending
            ? t.status.pending
            : isRunning
              ? t.status.stop
              : t.status.start}
        </Button>
      </div>
    </FieldGroup>
  );
}

function RecentFramesInspector() {
  const { t } = useTranslation();
  // 24-hour window. Limit caps the worst-case wire payload — a heavy
  // capture day at 5s interval is ~17k frames, but for a "your last
  // day" surface we only need enough to show the trend.
  const oneDayAgo = useMemo(() => {
    const d = new Date(Date.now() - 24 * 60 * 60 * 1000);
    return d.toISOString();
  }, []);
  const frames = useQuery({
    queryKey: [...RECENT_FRAMES_KEY, oneDayAgo],
    queryFn: () => framesList({ from: oneDayAgo, limit: 20000 }),
    staleTime: 60_000,
  });

  const stats = useMemo(() => {
    const list = frames.data ?? [];
    const buckets = new Array<number>(24).fill(0);
    let bytes = 0;
    for (const frame of list) {
      const captured = new Date(frame.captured_at);
      if (Number.isNaN(captured.getTime())) continue;
      const hourIndex = bucketIndex(captured);
      if (hourIndex !== null) {
        buckets[hourIndex] = (buckets[hourIndex] ?? 0) + 1;
      }
      bytes += frame.screenshot_size_bytes ?? 0;
    }
    const peak = peakHour(buckets);
    return {
      frames: list.length,
      bytes,
      buckets,
      peakHour: peak,
    };
  }, [frames.data]);

  if (frames.isLoading) {
    return (
      <FieldGroup title={t.settings.capture.recent.title}>
        <SettingsSkeleton />
      </FieldGroup>
    );
  }

  const peakLabel =
    stats.peakHour !== null
      ? `${String(stats.peakHour).padStart(2, "0")}:00`
      : "—";
  const max = Math.max(1, ...stats.buckets);

  return (
    <FieldGroup title={t.settings.capture.recent.title}>
      <div className="rounded-md border border-border bg-card px-4 py-4">
        <div className="flex items-baseline justify-between">
          <p className="text-sm font-medium text-foreground">
            {t.settings.capture.recent.subtitle(stats.frames, formatBytes(stats.bytes))}
          </p>
          <span className="text-[11px] text-muted-foreground">
            {t.settings.capture.recent.window}
          </span>
        </div>

        <div className="mt-3 grid grid-cols-3 gap-px overflow-hidden rounded-sm border border-border bg-border">
          <Stat label={t.settings.capture.recent.framesLabel}>
            {compactCount(stats.frames)}
          </Stat>
          <Stat label={t.settings.capture.recent.sizeLabel}>
            {formatBytes(stats.bytes)}
          </Stat>
          <Stat label={t.settings.capture.recent.peakLabel}>{peakLabel}</Stat>
        </div>

        <div className="mt-3 flex h-7 items-end gap-[2px]">
          {stats.buckets.map((count, idx) => {
            const ratio = count / max;
            const height = `${Math.max(6, Math.round(ratio * 100))}%`;
            const isPeak = stats.peakHour !== null && idx === stats.peakHour;
            return (
              <div
                key={idx}
                title={`${String(idx).padStart(2, "0")}:00 · ${count}`}
                className={`flex-1 rounded-[1px] ${
                  isPeak
                    ? "bg-[var(--corivo-amber)]"
                    : "bg-[color-mix(in_oklab,var(--corivo-amber)_25%,transparent)]"
                }`}
                style={{ height }}
              />
            );
          })}
        </div>
        <p className="mt-2 text-[10px] uppercase tracking-wider text-muted-foreground">
          {t.settings.capture.recent.histLegend}
        </p>
      </div>
    </FieldGroup>
  );
}

function Stat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="bg-card px-3 py-2.5">
      <div className="font-display text-lg font-semibold tracking-tight text-foreground">
        {children}
      </div>
      <div className="mt-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
    </div>
  );
}

/**
 * Tabbed exclusion card: Apps + Websites. Sits behind the privacy
 * popover's "Manage" button. Apps cover the bundle-id blocklist;
 * Websites cover host patterns (`example.com` or `*.example.com`)
 * applied to the URL that the AX path will eventually surface from
 * browsers — see the TODO marker in
 * `src-tauri/src/services/capture_pipeline/mod.rs` for the wiring
 * status.
 */
function ExclusionTabsCard() {
  const { t } = useTranslation();
  return (
    <FieldGroup title={t.settings.capture.exclusion.title}>
      <p className="mb-3 text-xs text-muted-foreground">
        {t.settings.capture.exclusion.explanation}
      </p>
      <Tabs defaultValue="apps" className="w-full">
        <TabsList className="grid w-full max-w-[260px] grid-cols-2">
          <TabsTrigger value="apps">
            {t.settings.capture.exclusion.appsTab}
          </TabsTrigger>
          <TabsTrigger value="websites">
            {t.settings.capture.exclusion.websitesTab}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="apps" className="space-y-3">
          <AppExclusionList />
        </TabsContent>
        <TabsContent value="websites" className="space-y-3">
          <WebsiteExclusionList />
        </TabsContent>
      </Tabs>
    </FieldGroup>
  );
}

function AppExclusionList() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const list = useQuery({
    queryKey: EXCLUSION_KEY,
    queryFn: exclusionList,
  });
  const [draft, setDraft] = useState("");

  const addMutation = useMutation({
    mutationFn: (bundleId: string) => exclusionAdd(bundleId),
    onSuccess: () => {
      setDraft("");
      void queryClient.invalidateQueries({ queryKey: EXCLUSION_KEY });
    },
    onError: (error: unknown) => {
      toast.error(t.common.addFailed(fromInvokeError(error)));
    },
  });
  const removeMutation = useMutation({
    mutationFn: (bundleId: string) => exclusionRemove(bundleId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: EXCLUSION_KEY });
    },
    onError: (error: unknown) => {
      toast.error(t.common.deleteFailed(fromInvokeError(error)));
    },
  });

  if (list.isLoading) {
    return <SettingsSkeleton />;
  }

  const entries = list.data ?? [];
  const defaults = entries.filter((entry) => entry.source === "default");
  const userEntries = entries.filter((entry) => entry.source === "user");

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label className="text-xs uppercase tracking-wide text-muted-foreground">
          {t.settings.capture.exclusion.defaultLabel}
        </Label>
        <div className="flex flex-wrap gap-1">
          {defaults.length === 0 ? (
            <span className="text-xs text-muted-foreground">
              {t.common.empty}
            </span>
          ) : (
            defaults.map((entry) => (
              <span
                key={entry.bundle_id}
                className="rounded-md bg-muted px-2 py-1 font-mono text-[11px] text-muted-foreground"
              >
                {entry.bundle_id}
              </span>
            ))
          )}
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase tracking-wide text-muted-foreground">
          {t.settings.capture.exclusion.userLabel}
        </Label>
        {userEntries.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            {t.settings.capture.exclusion.userEmpty}
          </p>
        ) : (
          <ul className="space-y-1">
            {userEntries.map((entry) => (
              <li
                key={entry.bundle_id}
                className="flex items-center justify-between rounded-md bg-muted/40 px-3 py-1.5"
              >
                <span className="font-mono text-sm">{entry.bundle_id}</span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => removeMutation.mutate(entry.bundle_id)}
                  disabled={removeMutation.isPending}
                >
                  {t.common.remove}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <Input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={t.settings.capture.exclusion.addPlaceholder}
          className="font-mono"
          disabled={addMutation.isPending}
        />
        <Button
          size="sm"
          onClick={() => addMutation.mutate(draft.trim())}
          disabled={draft.trim().length === 0 || addMutation.isPending}
        >
          {addMutation.isPending
            ? t.settings.capture.exclusion.adding
            : t.common.add}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {t.settings.capture.exclusion.bundleHint}
      </p>
    </div>
  );
}

/**
 * Websites tab — visible but inert until the capture pipeline can read
 * the active browser tab's URL from AX. The backend (service, Config
 * field, Tauri CRUD commands) is wired so the moment URL extraction
 * lights up, this tab swaps to a fully functional list with no
 * frontend changes beyond removing the disabled flag.
 *
 * We deliberately don't render the persisted patterns list while in
 * "Coming soon" mode — showing rows the user added in some earlier
 * build would imply they're active when they're not. Keeping the
 * input + Add button visible (just disabled) preserves the affordance
 * so the user can see what the eventual interaction looks like.
 */
function WebsiteExclusionList() {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="flex items-start gap-3 rounded-md border border-border bg-muted/40 px-3 py-2.5">
        <Clock className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <span className="text-[12.5px] font-semibold text-foreground/90">
              {t.settings.capture.exclusion.websitesComingSoonBadge}
            </span>
          </div>
          <p className="text-[12px] leading-[1.5] text-muted-foreground">
            {t.settings.capture.exclusion.websitesComingSoonBody}
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-2 opacity-60 sm:flex-row sm:items-center">
        <Input
          value=""
          readOnly
          placeholder={t.settings.capture.exclusion.websitesAddPlaceholder}
          className="font-mono"
          disabled
        />
        <Button size="sm" disabled>
          {t.common.add}
        </Button>
      </div>
    </div>
  );
}

function PermissionsCard() {
  const { t } = useTranslation();
  const systemInfo = useQuery({
    queryKey: ["system-info"],
    queryFn: getSystemInfo,
  });
  const screenRecording = useQuery({
    queryKey: ["screen-permission"],
    queryFn: checkScreenRecordingPermission,
    refetchInterval: 3000,
  });
  const ax = useQuery({
    queryKey: ["ax-permission"],
    queryFn: checkAxPermission,
    refetchInterval: 3000,
  });
  const restartMutation = useMutation({
    mutationFn: restartAsAdministrator,
    onMutate: () => toast.message(t.settings.permissions.adminRestart.starting),
    onError: (error) => toast.error(t.common.setupFailed(fromInvokeError(error))),
  });
  const isWindows = systemInfo.data?.os === "windows";

  return (
    <FieldGroup title={t.settings.permissions.title}>
      <p className="text-xs text-muted-foreground">
        {t.settings.permissions.description}
      </p>
      <div className="space-y-2">
        <PermissionRow
          title={t.settings.permissions.ax.title}
          description={t.settings.permissions.ax.description}
          granted={ax.data ?? false}
          loading={ax.isLoading}
          onOpenSettings={() => void openAxSettings()}
          required
        />
        <PermissionRow
          title={t.settings.permissions.screenRecording.title}
          description={t.settings.permissions.screenRecording.description}
          granted={screenRecording.data ?? false}
          loading={screenRecording.isLoading}
          onOpenSettings={() => void openSystemSettingsPrivacy()}
          required={false}
        />
        {isWindows && (
          <AdminRestartRow
            pending={restartMutation.isPending}
            onRestart={() => restartMutation.mutate()}
          />
        )}
      </div>
    </FieldGroup>
  );
}

function AdminRestartRow({
  pending,
  onRestart,
}: {
  pending: boolean;
  onRestart: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-md border border-border bg-card px-4 py-3">
      <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-[var(--color-amber-text)]" />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">
          {t.settings.permissions.adminRestart.title}
        </div>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          {t.settings.permissions.adminRestart.description}
        </p>
      </div>
      <Button
        size="sm"
        variant="secondary"
        className="shrink-0"
        disabled={pending}
        onClick={onRestart}
      >
        {pending
          ? t.status.pending
          : t.settings.permissions.adminRestart.button}
      </Button>
    </div>
  );
}

function PermissionRow({
  title,
  description,
  granted,
  loading,
  onOpenSettings,
  required,
}: {
  title: string;
  description: string;
  granted: boolean;
  loading: boolean;
  onOpenSettings: () => void;
  required: boolean;
}) {
  const { t } = useTranslation();
  const icon = granted ? (
    <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-700" />
  ) : required ? (
    <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-[var(--color-amber-text)]" />
  ) : (
    <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />
  );

  const status = statusLabel({ granted, loading, required }, t);

  return (
    <div className="flex items-start gap-3 rounded-md border border-border bg-card px-4 py-3">
      {icon}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">{title}</span>
          <span className="text-xs text-muted-foreground">· {status}</span>
        </div>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      {!granted && (
        <Button
          size="sm"
          variant="secondary"
          className="shrink-0 gap-1.5"
          onClick={onOpenSettings}
        >
          <ExternalLink className="h-3 w-3" />
          {t.settings.permissions.openSettings}
        </Button>
      )}
    </div>
  );
}

function statusLabel(
  { granted, loading, required }: {
    granted: boolean;
    loading: boolean;
    required: boolean;
  },
  t: LocaleDict,
): string {
  if (loading) return t.settings.permissions.checking;
  if (granted) return t.settings.permissions.granted;
  return required
    ? t.settings.permissions.requiredMissing
    : t.settings.permissions.optionalMissing;
}

function bucketIndex(d: Date): number | null {
  const h = d.getHours();
  if (Number.isNaN(h)) return null;
  return Math.min(23, Math.max(0, h));
}

function peakHour(buckets: number[]): number | null {
  let max = 0;
  let idx: number | null = null;
  for (let i = 0; i < buckets.length; i += 1) {
    const v = buckets[i] ?? 0;
    if (v > max) {
      max = v;
      idx = i;
    }
  }
  return max > 0 ? idx : null;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function compactCount(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}
