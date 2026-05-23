import { describe, expect, it } from "vitest"
import { router } from "@/app/router"

describe("router", () => {
  it("registers only the post-redesign top-level routes", () => {
    const registeredPaths = Object.keys(router.routesByPath)

    expect(registeredPaths).toContain("/")
    expect(registeredPaths).toContain("/ask")
    expect(registeredPaths).toContain("/login")
    expect(registeredPaths).toContain("/onboarding")
    // v3 onboarding (docs/design/onboarding-redesign-v2.html). The
    // 3 steps are Permission → Demo → Shortcut. Legacy step paths
    // (welcome / done / api-keys / warmup / try-it) are all gone.
    expect(registeredPaths).toContain("/onboarding/permission")
    expect(registeredPaths).toContain("/onboarding/demo")
    expect(registeredPaths).toContain("/onboarding/shortcut")
    expect(registeredPaths).not.toContain("/onboarding/welcome")
    expect(registeredPaths).not.toContain("/onboarding/done")
    expect(registeredPaths).not.toContain("/onboarding/api-keys")
    expect(registeredPaths).not.toContain("/onboarding/warmup")
    expect(registeredPaths).not.toContain("/onboarding/try-it")
  })

  it("does not expose retired routes (/timeline, /settings)", () => {
    // Timeline was retired in the redesign — see
    // docs/design/ia-v0.html. /settings has always been a dialog.
    const registeredPaths = Object.keys(router.routesByPath)
    expect(registeredPaths).not.toContain("/timeline")
    expect(registeredPaths).not.toContain("/settings")
  })
})
