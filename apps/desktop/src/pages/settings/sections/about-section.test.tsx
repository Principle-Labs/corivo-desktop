// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/i18n";
import { AboutSection } from "@/pages/settings/sections/about-section";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const {
  useQueryMock,
  useCapabilitiesMock,
  checkForUpdatesMock,
  installUpdateMock,
  updaterSnapshot,
} = vi.hoisted(() => ({
  useQueryMock: vi.fn(),
  useCapabilitiesMock: vi.fn(),
  checkForUpdatesMock: vi.fn(),
  installUpdateMock: vi.fn(),
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

vi.mock("@tanstack/react-query", () => ({
  useQuery: useQueryMock,
}));

vi.mock("@/hooks/use-capabilities", () => ({
  useCapabilities: useCapabilitiesMock,
}));

vi.mock("@/stores/updater-store", () => ({
  useUpdaterStore: (selector: (state: object) => unknown) =>
    selector({
      ...updaterSnapshot,
      checkForUpdates: checkForUpdatesMock,
      installUpdate: installUpdateMock,
    }),
}));

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
  updaterSnapshot.status = "idle";
  updaterSnapshot.availableVersion = null;
  updaterSnapshot.error = null;
  updaterSnapshot.progress.downloadedBytes = 0;
  updaterSnapshot.progress.totalBytes = 0;
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

async function renderAboutSection() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);

  // Pin the dictionary to 中文 — the production fallback now follows
  // the host's OS locale (English on most CI machines), so without
  // this wrapper the assertions below would compare 中文 expectations
  // against English rendered output.
  await act(async () => {
    root?.render(
      <I18nProvider language="zh">
        <AboutSection />
      </I18nProvider>
    );
  });
}

describe("AboutSection", () => {
  it("shows an 更新 button beside the current version when an update is available", async () => {
    updaterSnapshot.status = "available";
    updaterSnapshot.availableVersion = "0.2.0";

    await renderAboutSection();

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container?.textContent).toContain("应用版本：0.1.0");
    expect(container?.textContent).toContain("更新");
  });

  it("checks for updates from the version row when no update is pending", async () => {
    await renderAboutSection();

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    const button = Array.from(container?.querySelectorAll("button") ?? []).find(
      (item) => item.textContent?.includes("检查更新")
    );

    expect(button).toBeDefined();

    act(() => {
      button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(checkForUpdatesMock).toHaveBeenCalledTimes(1);
    expect(installUpdateMock).not.toHaveBeenCalled();
  });

  it("hides updater controls when the build has no managed updater", async () => {
    useCapabilitiesMock.mockReturnValue({
      data: {
        auth: false,
        billing: false,
        modelsDirectory: false,
        connectors: false,
        telemetry: false,
        managedUpdater: false,
      },
    });

    await renderAboutSection();

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container?.textContent).toContain("应用版本：0.1.0");
    expect(container?.textContent).not.toContain("检查更新");
  });
});
