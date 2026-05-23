//! Cross-context clipboard helper.
//!
//! Prefers the Tauri `clipboard-manager` plugin, which talks to the
//! native pasteboard and doesn't need a secure-context browser API.
//! Outside of Tauri (tests, non-Tauri builds) we fall back to the
//! web Clipboard API and finally to `document.execCommand("copy")`.

import { writeText as tauriWriteText } from "@tauri-apps/plugin-clipboard-manager";

function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  // Tauri v2 sets `__TAURI_INTERNALS__` on window; `isTauri` is also
  // exposed but not typed here. Either signal is enough.
  return "__TAURI_INTERNALS__" in window;
}

export async function copyText(text: string): Promise<void> {
  if (isTauri()) {
    await tauriWriteText(text);
    return;
  }

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through to the legacy path.
    }
  }

  if (typeof document === "undefined") {
    throw new Error("clipboard unavailable: no document");
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.top = "-9999px";
  textarea.style.left = "-9999px";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);

  const selection = document.getSelection();
  const previousRange =
    selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;

  textarea.focus();
  textarea.select();

  let succeeded = false;
  try {
    succeeded = document.execCommand("copy");
  } finally {
    document.body.removeChild(textarea);
    if (previousRange && selection) {
      selection.removeAllRanges();
      selection.addRange(previousRange);
    }
  }

  if (!succeeded) {
    throw new Error("clipboard write rejected by the platform");
  }
}
