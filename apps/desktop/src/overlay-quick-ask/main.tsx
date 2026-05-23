import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";

import { I18nProvider } from "@/i18n";
import { queryClient } from "@/lib/query-client";
import { QuickAskWindow } from "@/overlay-quick-ask/QuickAskWindow";
// `globals.css` first so Tailwind utilities + design tokens are
// available to MessageBubble; `quick-ask.css` after so the overlay
// chrome (transparent body, vibrancy, dark token overrides) wins.
import "@repo/ui/styles/globals.css";
import "@/styles/quick-ask.css";

document.body.dataset.quickAskRoot = "true";

// Quick Ask shares `useChatStream` with `/ask`, so it needs its own
// QueryClientProvider — Tauri webview windows are separate JS runtimes,
// so the main window's provider doesn't reach this root. The `queryClient`
// import resolves to a fresh instance per webview, which is what we want
// (per-window query cache; cross-window state goes through Tauri IPC).
//
// I18nProvider sits inside QueryClient so it can read the language
// preference from `Config.app.ui_language` via useConfig().
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <I18nProvider>
        <QuickAskWindow />
      </I18nProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
