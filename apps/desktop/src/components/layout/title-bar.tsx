// Absolute-positioned, zero-layout drag region. Sits on top of the window
// as a transparent overlay so macOS traffic lights have a hit target and
// the user can drag the window — but it does NOT push the underlying
// content down. Content fills h-screen edge to edge.
export function TitleBar() {
  return (
    <header
      data-testid="app-title-bar"
      data-tauri-drag-region=""
      className="pointer-events-auto absolute inset-x-0 top-0 z-50 h-6 select-none"
    />
  )
}
