import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";
import {
  attemptAppRelaunch,
  checkForAppUpdate,
  describeError,
  installAppUpdate,
  type AppUpdateHandle,
  type UpdateProgress,
} from "@/lib/updater";
import { logFrontend } from "@/lib/tauri";

type UpdaterDeps = {
  checkForAppUpdate: typeof checkForAppUpdate;
  installAppUpdate: typeof installAppUpdate;
  attemptAppRelaunch: typeof attemptAppRelaunch;
};

// 触发 relaunch 后给当前进程的兜底窗口：到点还活着说明 spawn 没拉起新进程，
// 此时把 status 切到 awaiting-restart，提示用户手动重启。
const RELAUNCH_FALLBACK_MS = 5000;

export type UpdaterStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "updating"
  | "restarting"
  // download + 验签 + 替换 .app 都成功，但 relaunch reject。.app 已经更新到磁盘，
  // 用户手动退出后重启即可拿到新版。
  | "awaiting-restart"
  | "error";

type AppUpdaterState = {
  status: UpdaterStatus;
  availableVersion: string | null;
  releaseDate: string | null;
  releaseNotes: string | null;
  error: string | null;
  progress: UpdateProgress;
  pendingUpdate: AppUpdateHandle | null;
  checkForUpdates: () => Promise<boolean>;
  installUpdate: () => Promise<boolean>;
};

const EMPTY_PROGRESS: UpdateProgress = {
  downloadedBytes: 0,
  totalBytes: 0,
};

const LOG_TARGET = "updater.store";

/**
 * Lift an unknown rejection to a stable string. Returns `null` when the
 * underlying value carries no useful detail — consumers are expected to
 * substitute a localized fallback (e.g. `t.updater.failedFallback`) so
 * the displayed copy follows `Config.app.ui_language`.
 */
function toErrorMessage(error: unknown): string | null {
  const detail = describeError(error);
  return detail && detail !== "[object Object]" ? detail : null;
}

export function createUpdaterStore(
  deps: UpdaterDeps = {
    checkForAppUpdate,
    installAppUpdate,
    attemptAppRelaunch,
  },
) {
  return createStore<AppUpdaterState>((set, get) => ({
    status: "idle",
    availableVersion: null,
    releaseDate: null,
    releaseNotes: null,
    error: null,
    progress: EMPTY_PROGRESS,
    pendingUpdate: null,
    async checkForUpdates() {
      const currentStatus = get().status;
      if (currentStatus === "checking" || currentStatus === "updating") {
        return Boolean(get().pendingUpdate);
      }

      set({
        status: "checking",
        error: null,
        progress: EMPTY_PROGRESS,
      });

      try {
        const update = await deps.checkForAppUpdate();

        if (!update) {
          set({
            status: "up-to-date",
            availableVersion: null,
            releaseDate: null,
            releaseNotes: null,
            pendingUpdate: null,
            error: null,
            progress: EMPTY_PROGRESS,
          });
          return false;
        }

        set({
          status: "available",
          availableVersion: update.version,
          releaseDate: update.date ?? null,
          releaseNotes: update.body ?? null,
          pendingUpdate: update,
          error: null,
          progress: EMPTY_PROGRESS,
        });
        return true;
      } catch (error) {
        const message = toErrorMessage(error);
        logFrontend("error", LOG_TARGET, "check_for_updates_threw", {
          error: describeError(error),
        });
        set({
          status: "error",
          availableVersion: null,
          releaseDate: null,
          releaseNotes: null,
          pendingUpdate: null,
          error: message,
          progress: EMPTY_PROGRESS,
        });
        return false;
      }
    },
    async installUpdate() {
      let update = get().pendingUpdate;

      if (!update) {
        const hasUpdate = await get().checkForUpdates();
        if (!hasUpdate) {
          return false;
        }
        update = get().pendingUpdate;
      }

      if (!update) {
        return false;
      }

      set({
        status: "updating",
        error: null,
        progress: EMPTY_PROGRESS,
      });

      try {
        await deps.installAppUpdate(update, (progress) => {
          set({ progress });
        });
      } catch (error) {
        // 真正的"装包失败"：下载/验签/替换 .app 那一段抛错。
        logFrontend("error", LOG_TARGET, "install_threw", {
          error: describeError(error),
        });
        set({
          status: "error",
          error: toErrorMessage(error),
        });
        return false;
      }

      // 装包成功——磁盘已是新版。立刻切到 restarting，触发 relaunch 但不 await
      // （IPC 必然 reject，见 `attemptAppRelaunch` 的注释）。如果 relaunch 真
      // 拉起新进程，当前进程会在几百毫秒内被杀，下面的 setTimeout 永远走不到；
      // 走到了就说明 spawn 没成功，让用户手动重启。
      set({
        status: "restarting",
        error: null,
        pendingUpdate: null,
      });
      deps.attemptAppRelaunch();
      setTimeout(() => {
        if (get().status === "restarting") {
          logFrontend(
            "warn",
            LOG_TARGET,
            "relaunch_did_not_kill_process_in_time",
            { timeout_ms: RELAUNCH_FALLBACK_MS },
          );
          set({ status: "awaiting-restart" });
        }
      }, RELAUNCH_FALLBACK_MS);
      return true;
    },
  }));
}

export const updaterStore = createUpdaterStore();

export function useUpdaterStore<T>(selector: (state: AppUpdaterState) => T): T {
  return useStore(updaterStore, selector);
}
