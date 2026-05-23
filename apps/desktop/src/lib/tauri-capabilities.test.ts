import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

type CapabilityConfig = {
  windows: string[];
  permissions: string[];
};

function loadDefaultCapability(): CapabilityConfig {
  const path = fileURLToPath(
    new URL("../../src-tauri/capabilities/default.json", import.meta.url),
  );
  return JSON.parse(readFileSync(path, "utf8")) as CapabilityConfig;
}

describe("tauri default capability", () => {
  it("allows notification overlay window geometry commands", () => {
    const capability = loadDefaultCapability();

    expect(capability.windows).toContain("notification-overlay");
    expect(capability.permissions).toContain("core:window:allow-hide");
    expect(capability.permissions).toContain("core:window:allow-show");
    expect(capability.permissions).toContain("core:window:allow-set-size");
    expect(capability.permissions).toContain("core:window:allow-set-position");
  });
});
