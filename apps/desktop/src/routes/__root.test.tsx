// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import type React from "react";

const { authStatusMock, getCapabilitiesMock } = vi.hoisted(() => ({
  authStatusMock: vi.fn(),
  getCapabilitiesMock: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  authStatus: authStatusMock,
  getCapabilities: getCapabilitiesMock,
}));

vi.mock("@/app/layout", () => ({
  AppLayout: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@tanstack/react-router-devtools", () => ({
  TanStackRouterDevtools: () => null,
}));

import { Route } from "./__root";

const OSS_CAPABILITIES = {
  auth: false,
  billing: false,
  modelsDirectory: false,
  connectors: false,
  telemetry: false,
  managedUpdater: false,
};

describe("root route auth guard", () => {
  it("rejects /login when auth capability is unavailable", async () => {
    getCapabilitiesMock.mockResolvedValue(OSS_CAPABILITIES);

    await expect(
      Route.options.beforeLoad?.({
        location: { pathname: "/login" },
      } as never),
    ).rejects.toBeDefined();
    expect(authStatusMock).not.toHaveBeenCalled();
  });

  it("allows app routes without auth status in OSS builds", async () => {
    getCapabilitiesMock.mockResolvedValue(OSS_CAPABILITIES);

    await expect(
      Route.options.beforeLoad?.({
        location: { pathname: "/ask" },
      } as never),
    ).resolves.toBeUndefined();
    expect(authStatusMock).not.toHaveBeenCalled();
  });
});
