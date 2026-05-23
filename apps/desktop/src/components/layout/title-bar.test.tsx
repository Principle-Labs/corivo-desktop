// @vitest-environment jsdom

import { act } from "react"
import { createRoot, type Root } from "react-dom/client"
import { afterEach, describe, expect, it } from "vitest"
import { TitleBar } from "@/components/layout/title-bar"

let container: HTMLDivElement | null = null
let root: Root | null = null

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  container?.remove()
  container = null
  root = null
})

describe("TitleBar", () => {
  it("renders a draggable title bar shell", () => {
    container = document.createElement("div")
    document.body.appendChild(container)
    root = createRoot(container)

    act(() => {
      root?.render(<TitleBar />)
    })

    const dragRegion = container.querySelector("[data-testid='app-title-bar']")
    expect(dragRegion).not.toBeNull()
    expect(dragRegion?.getAttribute("data-tauri-drag-region")).not.toBeNull()
  })
})
