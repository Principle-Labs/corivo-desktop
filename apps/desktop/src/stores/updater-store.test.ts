import { beforeEach, describe, expect, it, vi } from "vitest";
import { createUpdaterStore } from "@/stores/updater-store";

function createUpdate(version = "0.2.0") {
  return {
    version,
    date: "2026-04-14",
    body: "Bug fixes",
    downloadAndInstall: vi.fn(),
  };
}

describe("updater store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("records update metadata when a newer version is available", async () => {
    const update = createUpdate();
    const store = createUpdaterStore({
      checkForAppUpdate: vi.fn().mockResolvedValue(update),
      installAppUpdate: vi.fn(),
      attemptAppRelaunch: vi.fn(),
    });

    await expect(store.getState().checkForUpdates()).resolves.toBe(true);

    expect(store.getState()).toMatchObject({
      status: "available",
      availableVersion: "0.2.0",
      releaseDate: "2026-04-14",
      releaseNotes: "Bug fixes",
      error: null,
    });
  });

  it("keeps the pending update so installation can be retried after a failure", async () => {
    const update = createUpdate("0.3.0");
    const store = createUpdaterStore({
      checkForAppUpdate: vi.fn().mockResolvedValue(update),
      installAppUpdate: vi.fn().mockRejectedValue(new Error("network failed")),
      attemptAppRelaunch: vi.fn(),
    });

    await store.getState().checkForUpdates();
    await expect(store.getState().installUpdate()).resolves.toBe(false);

    expect(store.getState()).toMatchObject({
      status: "error",
      availableVersion: "0.3.0",
      error: "network failed",
    });
  });

  it("triggers relaunch (fire-and-forget) and falls back to awaiting-restart if process survives", async () => {
    vi.useFakeTimers();
    try {
      const update = createUpdate("0.4.0");
      const relaunchSpy = vi.fn();
      const store = createUpdaterStore({
        checkForAppUpdate: vi.fn().mockResolvedValue(update),
        installAppUpdate: vi.fn().mockResolvedValue(undefined),
        attemptAppRelaunch: relaunchSpy,
      });

      await store.getState().checkForUpdates();
      await expect(store.getState().installUpdate()).resolves.toBe(true);

      // Relaunch was triggered; status reflects the optimistic restart.
      expect(relaunchSpy).toHaveBeenCalledTimes(1);
      expect(store.getState().status).toBe("restarting");

      // After the fallback window with the process still alive, the store
      // should drop into awaiting-restart so the UI can prompt the user.
      vi.advanceTimersByTime(5000);
      expect(store.getState().status).toBe("awaiting-restart");
    } finally {
      vi.useRealTimers();
    }
  });
});
