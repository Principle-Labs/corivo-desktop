// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const {
  navigateMock,
  getOnboardingStateMock,
  authStatusMock,
  authRefreshMock,
  checkForAppUpdateMock,
  installAppUpdateMock,
  getUpdatePolicyMock,
  getVersionMock,
  getCapabilitiesMock,
} = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  getOnboardingStateMock: vi.fn(),
  authStatusMock: vi.fn(),
  authRefreshMock: vi.fn(),
  checkForAppUpdateMock: vi.fn(),
  installAppUpdateMock: vi.fn(),
  getUpdatePolicyMock: vi.fn(),
  getVersionMock: vi.fn(),
  getCapabilitiesMock: vi.fn(),
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

vi.mock("@/lib/updater", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/updater")>("@/lib/updater");
  return {
    ...actual,
    checkForAppUpdate: checkForAppUpdateMock,
    installAppUpdate: installAppUpdateMock,
    getUpdatePolicy: getUpdatePolicyMock,
  };
});

import { AppBoot } from "@/app/app-boot";
import { updaterStore } from "@/stores/updater-store";

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

beforeEach(() => {
  vi.clearAllMocks();
  updaterStore.setState({
    status: "idle",
    availableVersion: null,
    releaseDate: null,
    releaseNotes: null,
    error: null,
    progress: {
      downloadedBytes: 0,
      totalBytes: 0,
    },
    pendingUpdate: null,
  });
  checkForAppUpdateMock.mockResolvedValue(null);
  getOnboardingStateMock.mockResolvedValue({
    completed: true,
    version: 1,
    current_version: 1,
    needs_onboarding: false,
  });
  authStatusMock.mockResolvedValue({ loggedIn: true });
  authRefreshMock.mockResolvedValue({ loggedIn: true });
  getVersionMock.mockResolvedValue("0.2.0");
  getUpdatePolicyMock.mockResolvedValue({
    channel: "stable",
    minVersion: "0.0.1",
    latestVersion: "0.2.0",
    updatedAt: "2026-01-01T00:00:00.000Z",
  });
  // Closed-Corivo capability default keeps the existing auth + updater
  // assertions unchanged after the cloud-capability split.
  getCapabilitiesMock.mockResolvedValue({
    auth: true,
    billing: true,
    modelsDirectory: true,
    connectors: true,
    telemetry: true,
    managedUpdater: true,
  });
});

describe("AppBoot selector stability", () => {
  it("renders with the real updater store without unstable snapshot warnings", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });

    await expect(
      act(async () => {
        root?.render(
          <QueryClientProvider client={queryClient!}>
            <AppBoot />
          </QueryClientProvider>,
        );
        await Promise.resolve();
        await Promise.resolve();
      }),
    ).resolves.toBeUndefined();

    const consoleOutput = errorSpy.mock.calls.flat().join("\n");
    expect(consoleOutput).not.toContain("getSnapshot should be cached");
    expect(consoleOutput).not.toContain("Maximum update depth exceeded");

    errorSpy.mockRestore();
  });
});
