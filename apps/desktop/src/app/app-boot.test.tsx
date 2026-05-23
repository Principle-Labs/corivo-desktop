// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppBoot } from "@/app/app-boot";
import { I18nProvider } from "@/i18n";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const {
  navigateMock,
  getOnboardingStateMock,
  authStatusMock,
  authRefreshMock,
  checkForUpdatesMock,
  installUpdateMock,
  getUpdatePolicyMock,
  getVersionMock,
  getCapabilitiesMock,
  updaterSnapshot,
} = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  getOnboardingStateMock: vi.fn(),
  authStatusMock: vi.fn(),
  authRefreshMock: vi.fn(),
  checkForUpdatesMock: vi.fn(),
  installUpdateMock: vi.fn(),
  getUpdatePolicyMock: vi.fn(),
  getVersionMock: vi.fn(),
  getCapabilitiesMock: vi.fn(),
  updaterSnapshot: {
    status: "idle",
    availableVersion: null as string | null,
    error: null as string | null,
    progress: {
      downloadedBytes: 0,
      totalBytes: 0,
    },
  },
}));

vi.mock("@tanstack/react-router", () => ({
  RouterProvider: () => <div data-testid="router-ready">router</div>,
}));

vi.mock("@/app/router", () => ({
  router: {
    navigate: navigateMock,
  },
}));

vi.mock("@/lib/tauri", () => ({
  getOnboardingState: getOnboardingStateMock,
  authStatus: authStatusMock,
  authRefresh: authRefreshMock,
  getCapabilities: getCapabilitiesMock,
}));

vi.mock("@/lib/updater", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/updater")>("@/lib/updater");
  return {
    ...actual,
    getUpdatePolicy: getUpdatePolicyMock,
  };
});

// AppBoot calls `listen()` for `corivo:auth_required` and
// `ask:open-thread`. Without this mock the real Tauri shim crashes
// in jsdom (`transformCallback` reads window.__TAURI_INTERNALS__).
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => undefined),
}));

// usePermissionListener (mounted at the AppBoot root) calls
// getCurrentWebviewWindow().listen — stub it so the hook is a no-op
// in jsdom rather than crashing on missing Tauri internals.
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({
    listen: vi.fn().mockResolvedValue(() => undefined),
  }),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: getVersionMock,
}));

vi.mock("@/stores/updater-store", () => ({
  useUpdaterStore: (selector: (state: object) => unknown) =>
    selector({
      ...updaterSnapshot,
      checkForUpdates: checkForUpdatesMock,
      installUpdate: installUpdateMock,
    }),
  updaterStore: {
    getState: () => ({
      checkForUpdates: checkForUpdatesMock,
      installUpdate: installUpdateMock,
    }),
  },
}));

let container: HTMLDivElement | null = null;
let root: Root | null = null;
let queryClient: QueryClient | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
  queryClient = null;
});

function renderApp() {
  queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  // Force the Chinese dictionary so the assertions below can match
  // 中文 strings — the production fallback now follows the host's OS
  // locale (English on most CI / dev machines), which would otherwise
  // make these tests host-locale-dependent.
  return (
    <QueryClientProvider client={queryClient}>
      <I18nProvider language="zh">
        <AppBoot />
      </I18nProvider>
    </QueryClientProvider>
  );
}

beforeEach(() => {
  updaterSnapshot.status = "idle";
  updaterSnapshot.availableVersion = null;
  updaterSnapshot.error = null;
  updaterSnapshot.progress.downloadedBytes = 0;
  updaterSnapshot.progress.totalBytes = 0;
  vi.clearAllMocks();
  // Default to a logged-in beta user so the auth gate stays out of the
  // way for tests that aren't explicitly about it. Individual tests can
  // override these mocks before rendering.
  authStatusMock.mockResolvedValue({ loggedIn: true });
  authRefreshMock.mockResolvedValue({ loggedIn: true });
  // Default policy: current version meets the floor. Tests that exercise
  // the force-update gate override this.
  getVersionMock.mockResolvedValue("0.2.0");
  getUpdatePolicyMock.mockResolvedValue({
    channel: "stable",
    minVersion: "0.0.1",
    latestVersion: "0.2.0",
    updatedAt: "2026-01-01T00:00:00.000Z",
  });
  // Default to the closed-Corivo capability snapshot so existing tests
  // (which exercise the auth + managed-updater paths) keep behaving the
  // way they did before the cloud-capability split landed. OSS-flavoured
  // tests should explicitly override with `CAPABILITIES_FALLBACK`-shaped
  // values.
  getCapabilitiesMock.mockResolvedValue({
    auth: true,
    billing: true,
    modelsDirectory: true,
    connectors: true,
    telemetry: true,
    managedUpdater: true,
  });
});

describe("AppBoot", () => {
  it("forces update when current version is below minVersion", async () => {
    getVersionMock.mockResolvedValue("0.1.0");
    getUpdatePolicyMock.mockResolvedValue({
      channel: "stable",
      minVersion: "0.2.0",
      latestVersion: "0.2.0",
      updatedAt: "2026-01-01T00:00:00.000Z",
    });
    updaterSnapshot.status = "updating";
    updaterSnapshot.availableVersion = "0.2.0";
    checkForUpdatesMock.mockResolvedValue(true);
    installUpdateMock.mockReturnValue(new Promise<boolean>(() => undefined));

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await act(async () => {
      root?.render(renderApp());
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container.textContent).toContain("正在更新");
    expect(container.querySelector("[data-testid='router-ready']")).toBeNull();
    expect(getOnboardingStateMock).not.toHaveBeenCalled();
  });

  it("continues boot when current version meets the minVersion floor", async () => {
    getVersionMock.mockResolvedValue("0.2.0");
    getUpdatePolicyMock.mockResolvedValue({
      channel: "stable",
      minVersion: "0.2.0",
      latestVersion: "0.3.0", // newer is available but not required
      updatedAt: "2026-01-01T00:00:00.000Z",
    });
    getOnboardingStateMock.mockResolvedValue({
      completed: true,
      version: 1,
      current_version: 1,
      needs_onboarding: false,
    });

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await act(async () => {
      root?.render(renderApp());
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container.querySelector("[data-testid='router-ready']")).not.toBeNull();
    expect(getOnboardingStateMock).toHaveBeenCalledTimes(1);
    // 不应该在启动路径上自动检查/安装,这交给设置页
    expect(checkForUpdatesMock).not.toHaveBeenCalled();
    expect(installUpdateMock).not.toHaveBeenCalled();
  });

  it("does not block boot when policy fetch returns null", async () => {
    getVersionMock.mockResolvedValue("0.2.0");
    getUpdatePolicyMock.mockResolvedValue(null);
    getOnboardingStateMock.mockResolvedValue({
      completed: true,
      version: 1,
      current_version: 1,
      needs_onboarding: false,
    });

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await act(async () => {
      root?.render(renderApp());
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(checkForUpdatesMock).not.toHaveBeenCalled();
    expect(container.querySelector("[data-testid='router-ready']")).not.toBeNull();
  });
});
