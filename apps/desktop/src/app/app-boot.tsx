import { useEffect, useState } from "react";
import { RouterProvider } from "@tanstack/react-router";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { useShallow } from "zustand/react/shallow";
import { FocusMark } from "@repo/ui/components/focus-mark";
import { Mascot } from "@/components/brand/mascot";
import { router } from "@/app/router";
import { useChatStreamMirror, useChatThreadsSync } from "@/hooks/use-chat";
import { useConfigSync } from "@/hooks/use-config";
import { usePermissionListener } from "@/hooks/use-permission-listener";
import { useTranslation } from "@/i18n";
import { authRefresh, authStatus, getCapabilities, getOnboardingState } from "@/lib/tauri";
import { CAPABILITIES_FALLBACK } from "@/hooks/use-capabilities";
import { compareVersions, getUpdatePolicy } from "@/lib/updater";
import { PermissionDialog } from "@/pages/ask/permission-dialog";
import { updaterStore, useUpdaterStore } from "@/stores/updater-store";
import { applyAuthStatus } from "@/stores/user-profile-store";

type BootMode = "loading" | "force-update" | "ready";

export function AppBoot() {
  const [mode, setMode] = useState<BootMode>("loading");
  const updater = useUpdaterStore(
    useShallow((state) => ({
      status: state.status,
      availableVersion: state.availableVersion,
      error: state.error,
      progress: state.progress,
    })),
  );
  // Listen to chat:threads-changed broadcasts from any window (Quick
  // Ask in particular) and refresh this window's thread list cache.
  // Mounted at the root so it covers /ask, /timeline, /settings, and
  // even the boot splash — listener is idle until consumers exist.
  useChatThreadsSync();
  // Mirror in-flight streams started in any other window into this
  // window's `useStreamingStore`, so a turn the user fires from Quick
  // Ask renders token-by-token in the main /ask view (and vice versa).
  useChatStreamMirror();
  // Same pattern for config:changed — keeps the I18nProvider in sync
  // when the user flips language from any window.
  useConfigSync();

  useEffect(() => {
    let cancelled = false;

    const init = async () => {
      const updaterState = updaterStore.getState();

      try {
        // 第一步：拿 capability。失败时取 OSS-style fallback（全部 false），
        // 这样接下来的更新检查 / 登录跳转一并跳过——比起阻断启动，把无云
        // 服务的用户挡在主界面外更糟。
        const capabilities = await getCapabilities().catch(() => CAPABILITIES_FALLBACK);

        if (cancelled) {
          return;
        }

        // 启动时只在低于 minVersion 时强制更新;其它"有新版"留给用户
        // 在设置页主动检查。policy 拉不到(API 故障/网络问题/dev 模式)
        // 直接放行——不让中央 API 故障锁死所有用户,丢失的强制升级机会
        // 由下次成功 boot 时补上。
        // capabilities.managedUpdater=false（开源构建）时彻底跳过 policy
        // 取数；getUpdatePolicy 自己已经会在 dev 模式返回 null，这里再加
        // 一道编译期判断把分支前移，更显式。
        const [policy, current] = await Promise.all([
          capabilities.managedUpdater ? getUpdatePolicy() : Promise.resolve(null),
          getVersion(),
        ]);
        const mustForceUpdate =
          policy !== null && compareVersions(current, policy.minVersion) < 0;

        if (cancelled) {
          return;
        }

        if (mustForceUpdate) {
          const hasUpdate = await updaterState.checkForUpdates();
          if (cancelled) {
            return;
          }
          setMode("force-update");
          if (hasUpdate) {
            await updaterState.installUpdate();
          }
          // hasUpdate 为 false 说明 policy 说要强制但 updater 没找到本平台的包,
          // 让用户停在阻断界面,显示 updaterStore 的 error/idle 状态。
          return;
        }

        // 闭源 cloud 路径：先看 session 是否在，若不在跳 /login。
        // 开源构建 capabilities.auth=false 时整块跳过——没有登录概念，
        // 用户直接进 onboarding / 主界面。
        if (capabilities.auth) {
          const initialAuth = await authStatus();
          if (cancelled) {
            return;
          }
          if (!initialAuth.loggedIn) {
            applyAuthStatus(null);
            await router.navigate({ to: "/login", replace: true });
            return;
          }
          try {
            // refresh() repopulates the in-memory cloud creds
            // (including the latest profile fields). We then
            // re-fetch the status snapshot so the local user-profile
            // store reflects whatever the server just told us.
            const refreshed = await authRefresh();
            applyAuthStatus(refreshed);
          } catch (refreshError) {
            console.warn("auth refresh failed at boot", refreshError);
            // session.refresh() in Rust only clears the on-disk token on
            // a 401 (via force_logout). Network errors / 5xx / parse
            // failures leave the token alone. Re-read the disk-backed
            // status: if the token survived, treat the boot as logged-in
            // and let the user proceed with the stale snapshot — the
            // corivo:auth_required listener below will route to /login
            // if a later cloud call truly 401s.
            const recheck = await authStatus().catch(() => null);
            if (cancelled) {
              return;
            }
            if (recheck?.loggedIn) {
              applyAuthStatus(recheck);
            } else {
              applyAuthStatus(null);
              await router.navigate({ to: "/login", replace: true });
              return;
            }
          }
        } else {
          // OSS：没有云端账户。用本地"游客"快照填充 user-profile-store
          // 让 sidebar / settings 渲染不空（避免一堆 undefined 分支）。
          applyAuthStatus(null);
        }

        const onboarding = await getOnboardingState();
        if (!cancelled && onboarding.needs_onboarding) {
          // v3 onboarding (docs/design/onboarding-redesign-v2.html).
          //
          // Resume points: `permission` and `demo` are persisted by
          // OnboardingLayout. `shortcut` is intentionally NOT
          // persisted — on force-quit we'd rather they re-see the
          // demo than land cold on the keyboard diagram.
          //
          // Legacy step names from earlier versions all coerce back
          // to the entry point so stale `onboarding_step` values
          // never 404. AX + Screen Recording status is queried fresh
          // anyway, so re-landing on permission is cheap (green
          // checks show immediately if grants are still in place).
          const stepRouteMap: Record<string, string> = {
            // v3 (current)
            permission: "/onboarding/permission",
            demo: "/onboarding/demo",
            // v3 — shortcut isn't persisted but tolerate the name
            // anyway in case some test fixture writes it.
            shortcut: "/onboarding/demo",
            // v2 legacy (warmup + try-it)
            warmup: "/onboarding/permission",
            "try-it": "/onboarding/permission",
            // v1 legacy
            welcome: "/onboarding/permission",
            done: "/onboarding/permission",
          };
          const target = onboarding.step
            ? (stepRouteMap[onboarding.step] ?? "/onboarding/permission")
            : "/onboarding/permission";
          await router.navigate({
            to: target as "/onboarding/permission" | "/onboarding/demo",
            replace: true,
          });
        }
      } catch (error) {
        console.error("failed to load onboarding state", error);
      } finally {
        if (!cancelled) {
          setMode((currentMode) =>
            currentMode === "force-update" ? currentMode : "ready"
          );
        }
      }
    };

    void init();

    // Backend-emitted: any unrecoverable cloud auth 401 kicks the user
    // back to /login. Mounted at the root so it works regardless of which page
    // first triggered the failure (chat / settings / quick-ask).
    const unlistenAuth = listen("corivo:auth_required", () => {
      applyAuthStatus(null);
      void router.navigate({ to: "/login", replace: true });
    });

    // Quick Ask "在 App 中查看" deep-link. The Quick Ask window calls
    // `quick_ask_open_in_app(threadId)`, the backend raises us +
    // emits this event with the thread id as payload. Routing here at
    // the root means it works no matter which page is currently up —
    // /ask reads `?threadId=…` from search and switches to that
    // thread on mount. Payload shape: bare string thread id.
    const unlistenOpenThread = listen<string>("ask:open-thread", (event) => {
      const threadId = event.payload;
      if (typeof threadId !== "string" || threadId.length === 0) return;
      void router.navigate({
        to: "/ask",
        search: { threadId },
        replace: false,
      });
    });

    return () => {
      cancelled = true;
      void unlistenAuth.then((unlisten) => unlisten());
      void unlistenOpenThread.then((unlisten) => unlisten());
    };
  }, []);

  if (mode !== "ready") {
    return (
      <BootSplash
        forceUpdate={mode === "force-update"}
        updaterStatus={updater.status}
        availableVersion={updater.availableVersion}
        error={updater.error}
        progress={updater.progress}
        onRetry={() => updaterStore.getState().installUpdate()}
      />
    );
  }

  return (
    <>
      <RouterProvider router={router} />
      <GlobalPermissionDialog />
    </>
  );
}

/**
 * Mounts the permission listener at the app root so any chat turn
 * (Quick Ask, /ask, future surfaces) can surface a tool-use approval
 * dialog without each page wiring its own listener.
 */
function GlobalPermissionDialog() {
  const { current, dequeue } = usePermissionListener();
  return <PermissionDialog request={current} onResolved={dequeue} />;
}

function BootSplash({
  forceUpdate,
  updaterStatus,
  availableVersion,
  error,
  progress,
  onRetry,
}: {
  forceUpdate: boolean;
  updaterStatus: string;
  availableVersion: string | null;
  error: string | null;
  progress: {
    downloadedBytes: number;
    totalBytes: number;
  };
  onRetry: () => Promise<boolean>;
}) {
  const { t } = useTranslation();
  if (forceUpdate) {
    const progressText =
      progress.totalBytes > 0
        ? t.app.forceUpdate.downloadProgress(
            formatBytes(progress.downloadedBytes),
            formatBytes(progress.totalBytes),
          )
        : null;

    const isAwaitingRestart = updaterStatus === "awaiting-restart";
    const isErrorLike = updaterStatus === "error" || isAwaitingRestart;

    return (
      <div className="flex h-screen items-center justify-center bg-background text-foreground">
        <div
          className="w-full max-w-sm space-y-5 rounded-[14px] border border-border bg-card p-7 text-center"
          style={{ boxShadow: "var(--shadow-md)" }}
        >
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-[12px] bg-[var(--accent)] text-[var(--accent-foreground)]">
            <FocusMark size={26} strokeWidth={1.5} />
          </div>
          <div className="space-y-1.5">
            <div className="font-display text-[17px] font-semibold tracking-[-0.01em] text-foreground">
              {isErrorLike
                ? t.app.forceUpdate.errorTitle
                : t.app.forceUpdate.title}
            </div>
            <div className="text-[13px] text-muted-foreground">
              {isAwaitingRestart
                ? t.updater.awaitingRestart
                : updaterStatus === "error"
                  ? (error ?? t.app.forceUpdate.errorFallback)
                  : availableVersion
                    ? t.app.forceUpdate.foundVersion(availableVersion)
                    : t.app.forceUpdate.preparing}
            </div>
          </div>
          {progressText ? (
            <div className="font-mono text-[11px] tracking-[0.04em] text-muted-foreground">
              {progressText}
            </div>
          ) : null}
          {updaterStatus === "error" ? (
            <button
              type="button"
              className="inline-flex h-9 items-center justify-center rounded-[6px] bg-foreground px-4 text-[13px] font-medium text-background transition-colors hover:bg-foreground/90"
              onClick={() => {
                void onRetry();
              }}
            >
              {t.app.forceUpdate.retry}
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    // Idle boot splash — calmer than the force-update card. Centered
    // brand mark over a breathing dot, paper background, no card chrome.
    // The mark uses the same Focus Mark wordmark used everywhere else,
    // so the boot transition feels like the app fading into itself
    // rather than a separate splash screen.
    <div className="relative flex h-screen flex-col items-center justify-center gap-5 bg-background text-foreground">
      <div className="flex flex-col items-center gap-3.5">
        <Mascot size="lg" breath />
        <span className="font-display text-[15px] font-semibold tracking-[-0.01em] text-foreground">
          Corivo
        </span>
      </div>
      <div className="flex items-center gap-2">
        <span
          className="h-1 w-1 rounded-full bg-[var(--corivo-amber)]"
          style={{ animation: "corivo-status-pulse 1.6s ease-in-out infinite" }}
        />
        <span className="text-[12px] tracking-[-0.005em] text-muted-foreground">
          {t.app.booting}
        </span>
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) {
    return "0 B";
  }

  if (bytes < 1024) {
    return `${bytes} B`;
  }

  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }

  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
