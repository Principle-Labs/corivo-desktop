import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { logFrontend } from "@/lib/tauri";

export type AppUpdateHandle = Update;

export interface UpdateProgress {
  downloadedBytes: number;
  totalBytes: number;
}

export interface UpdatePolicy {
  channel: string;
  minVersion: string;
  latestVersion: string;
  updatedAt: string;
}

// Tauri plugin 抛出的错误经常不是标准 Error 对象（可能是 `{ kind, source }` 结构），
// 直接 `String(e)` 会得到 `[object Object]`；这里把常见字段都铺平。
export function describeError(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    try {
      const json = JSON.stringify(error);
      if (json && json !== "{}") return json;
    } catch {
      // fall through
    }
  }
  return String(error);
}

const LOG_TARGET = "updater";

// OSS builds do not have a Corivo-managed update policy service. Forks
// that want a forced/min-version policy can opt in at build time.
const POLICY_URL = import.meta.env.VITE_CORIVO_UPDATE_POLICY_URL;

export async function getUpdatePolicy(): Promise<UpdatePolicy | null> {
  if (import.meta.env.DEV || !POLICY_URL) return null;
  try {
    const res = await fetch(POLICY_URL);
    if (!res.ok) {
      logFrontend("warn", LOG_TARGET, "policy_http_error", { status: res.status });
      return null;
    }
    return (await res.json()) as UpdatePolicy;
  } catch (error) {
    logFrontend("warn", LOG_TARGET, "policy_fetch_failed", {
      error: describeError(error),
    });
    return null;
  }
}

// 数字三段式 semver 比较;不处理 pre-release tag(我们的 release 不用)。
export function compareVersions(a: string, b: string): number {
  const pa = a.trim().split(".").map((s) => Number.parseInt(s, 10) || 0);
  const pb = b.trim().split(".").map((s) => Number.parseInt(s, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x - y;
  }
  return 0;
}

export async function checkForAppUpdate(): Promise<AppUpdateHandle | null> {
  // Dev 构建用 `ai.corivo.desktop.dev` identifier 且 updater
  // endpoints 在 tauri.dev.conf.json 里被清空了；这里再加一道
  // guard 避免运行时调用导致 store 进入 error 状态。
  if (import.meta.env.DEV) {
    return null;
  }
  logFrontend("info", LOG_TARGET, "check_started");
  try {
    const update = await check();
    if (update) {
      logFrontend("info", LOG_TARGET, "check_update_available", {
        version: update.version,
        date: update.date ?? null,
      });
    } else {
      logFrontend("info", LOG_TARGET, "check_up_to_date");
    }
    return update;
  } catch (error) {
    logFrontend("error", LOG_TARGET, "check_failed", {
      error: describeError(error),
    });
    throw error;
  }
}

// 只做 download + 验签 + 替换 .app。装完即返回，不再触发重启——重启
// 单独由 `attemptAppRelaunch` 负责，原因见函数注释。
export async function installAppUpdate(
  update: AppUpdateHandle,
  onProgress?: (progress: UpdateProgress) => void,
): Promise<void> {
  let downloadedBytes = 0;
  let totalBytes = 0;

  logFrontend("info", LOG_TARGET, "install_started", {
    version: update.version,
  });

  try {
    await update.downloadAndInstall((event: DownloadEvent) => {
      switch (event.event) {
        case "Started":
          downloadedBytes = 0;
          totalBytes = event.data.contentLength ?? 0;
          logFrontend("info", LOG_TARGET, "download_started", {
            content_length: totalBytes,
          });
          onProgress?.({ downloadedBytes, totalBytes });
          break;
        case "Progress":
          downloadedBytes += event.data.chunkLength;
          onProgress?.({ downloadedBytes, totalBytes });
          break;
        case "Finished":
          logFrontend("info", LOG_TARGET, "download_finished", {
            downloaded_bytes: totalBytes || downloadedBytes,
          });
          onProgress?.({
            downloadedBytes: totalBytes || downloadedBytes,
            totalBytes,
          });
          break;
      }
    });
  } catch (error) {
    logFrontend("error", LOG_TARGET, "install_failed", {
      error: describeError(error),
    });
    throw error;
  }

  logFrontend("info", LOG_TARGET, "install_finished");
}

// `tauri-plugin-process::relaunch` 的实现是「spawn 新进程后立即 exit(0)」。
// 这意味着即便重启**完全成功**，等待 IPC 响应的前端 Promise 也必然 reject——
// 因为响应包还没回到 webview 当前进程就被杀了。所以这里**故意不 await**：
// 调用即触发，由调用方用 setTimeout 兜底判断"是否仍然存活"。
//
// 唯一能从 reject 中读到的信号是 spawn 失败时抛出的 OS 错误（罕见）；
// 我们仍然 catch 一下记 warn 日志，但从不让它向上传播。
export function attemptAppRelaunch(): void {
  logFrontend("info", LOG_TARGET, "relaunch_attempt");
  try {
    void relaunch().catch((error) => {
      logFrontend("warn", LOG_TARGET, "relaunch_ipc_rejected", {
        error: describeError(error),
      });
    });
  } catch (error) {
    logFrontend("warn", LOG_TARGET, "relaunch_threw_sync", {
      error: describeError(error),
    });
  }
}
