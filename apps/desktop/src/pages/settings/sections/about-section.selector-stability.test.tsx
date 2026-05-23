// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const {
  useQueryMock,
  useCapabilitiesMock,
  checkForAppUpdateMock,
  installAppUpdateMock,
  attemptAppRelaunchMock,
} = vi.hoisted(() => ({
  useQueryMock: vi.fn(),
  useCapabilitiesMock: vi.fn(),
  checkForAppUpdateMock: vi.fn(),
  installAppUpdateMock: vi.fn(),
  attemptAppRelaunchMock: vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: useQueryMock,
}));

vi.mock("@/hooks/use-capabilities", () => ({
  useCapabilities: useCapabilitiesMock,
}));

vi.mock("@/lib/updater", () => ({
  checkForAppUpdate: checkForAppUpdateMock,
  installAppUpdate: installAppUpdateMock,
  attemptAppRelaunch: attemptAppRelaunchMock,
}));

import { AboutSection } from "@/pages/settings/sections/about-section";
import { updaterStore } from "@/stores/updater-store";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
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
  useQueryMock.mockReturnValue({
    isLoading: false,
    data: {
      os: "macOS",
      arch: "arm64",
      os_version: "14.0",
      app_version: "0.1.0",
      tauri_version: "2",
    },
  });
  useCapabilitiesMock.mockReturnValue({
    data: {
      auth: true,
      billing: true,
      modelsDirectory: true,
      connectors: true,
      telemetry: true,
      managedUpdater: true,
    },
  });
});

describe("AboutSection selector stability", () => {
  it("renders with the real updater store without unstable snapshot warnings", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await expect(
      act(async () => {
        root?.render(<AboutSection />);
        await Promise.resolve();
      }),
    ).resolves.toBeUndefined();

    const consoleOutput = errorSpy.mock.calls.flat().join("\n");
    expect(consoleOutput).not.toContain("getSnapshot should be cached");
    expect(consoleOutput).not.toContain("Maximum update depth exceeded");

    errorSpy.mockRestore();
  });
});
