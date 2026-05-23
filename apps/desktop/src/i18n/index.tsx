import { createContext, useContext, useEffect, useMemo, type ReactNode } from "react";
import { useConfig } from "@/hooks/use-config";
import type { Language } from "@/lib/types";
import { en } from "./locales/en";
import { zh, type LocaleDict } from "./locales/zh";

const DICTS: Record<Language, LocaleDict> = { zh, en };

interface I18nContextValue {
  /** The active dictionary — read keys directly: `t.nav.ask`. */
  t: LocaleDict;
  /** The active language. Use this when you need a branch other than
   *  what's already encoded in the dictionary. */
  lang: Language;
}

/// Cached UI-language key for the cold-open bootstrap. The Quick Ask
/// overlay is hidden by default and only shown on hotkey, so each open
/// is effectively a cold mount: `useConfig()` fires an IPC, briefly
/// returns `undefined`, and the provider would otherwise default to
/// Chinese for ~50–200 ms before the real value lands. Mirroring the
/// language to localStorage on every change lets the next mount read
/// it synchronously and start in the correct dictionary.
const LANG_STORAGE_KEY = "corivo:ui-language";

/// Read the host's primary UI language from `navigator.language` and
/// collapse it to the two locales Corivo currently ships (`zh` / `en`).
/// Used as the very-first-paint fallback before either localStorage or
/// the canonical config IPC has answered. Mirrors the Rust-side
/// `Language::detect_from_os()` mapping so cold mounts and the first
/// post-install config save agree on what to show.
function detectBrowserLanguage(): Language {
  if (typeof navigator === "undefined") return "en";
  const tag = navigator.language?.toLowerCase() ?? "";
  return tag.startsWith("zh") ? "zh" : "en";
}

function readCachedLanguage(): Language {
  if (typeof window === "undefined") return "en";
  try {
    const cached = window.localStorage.getItem(LANG_STORAGE_KEY);
    if (cached === "zh" || cached === "en") return cached;
  } catch {
    // localStorage can throw in Safari private mode etc. — fall back
    // silently rather than crashing the boot.
  }
  return detectBrowserLanguage();
}

function writeCachedLanguage(language: Language): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LANG_STORAGE_KEY, language);
  } catch {
    // Ignored — same reason as readCachedLanguage.
  }
}

/// Provider-less consumers (test harnesses, Storybook-style mounts)
/// land on this fallback. English is the global default; users who
/// never reach `<I18nProvider>` shouldn't end up in 中文 just because
/// it's the locale source-of-truth file.
const FALLBACK: I18nContextValue = { t: en, lang: "en" };

const I18nContext = createContext<I18nContextValue>(FALLBACK);

interface I18nProviderProps {
  children: ReactNode;
  /** Optional override; otherwise the active language is read from
   *  `Config.app.ui_language` via `useConfig()`. Used by tests and the
   *  overlay roots that may render before the config query resolves. */
  language?: Language;
}

/**
 * Top-level provider. Mounts inside each Tauri webview's React root —
 * `apps/desktop/src/main.tsx` (main + settings), `apps/desktop/src/overlay-quick-ask/main.tsx`
 * (Quick Ask). Subscribes to the persisted UI language via `useConfig()`.
 *
 * On cold open the config query is in flight, so we fall back to a
 * localStorage-cached language (`LANG_STORAGE_KEY`). The cache is
 * refreshed every time the canonical config returns a new value.
 *
 * If `language` is supplied (test harnesses, storybook-style mounts),
 * we skip `useConfig()` entirely — those callers shouldn't have to
 * mock `getConfig` / `useQueryClient` just to pin a dictionary.
 */
export function I18nProvider({ children, language }: I18nProviderProps) {
  if (language !== undefined) {
    return <I18nMount lang={language}>{children}</I18nMount>;
  }
  return <I18nFromConfig>{children}</I18nFromConfig>;
}

function I18nFromConfig({ children }: { children: ReactNode }) {
  const { config } = useConfig();
  const resolvedLang: Language =
    config?.app.ui_language ?? readCachedLanguage();

  // Mirror the canonical language back into localStorage so future cold
  // opens (Quick Ask hotkey, app restart) start in the correct dict.
  useEffect(() => {
    if (config?.app.ui_language) {
      writeCachedLanguage(config.app.ui_language);
    }
  }, [config?.app.ui_language]);

  return <I18nMount lang={resolvedLang}>{children}</I18nMount>;
}

function I18nMount({
  lang,
  children,
}: {
  lang: Language;
  children: ReactNode;
}) {
  const value = useMemo<I18nContextValue>(
    () => ({ t: DICTS[lang], lang }),
    [lang],
  );
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/**
 * Read the active dictionary + language. Returns the Chinese fallback
 * outside any provider (test harnesses without `<I18nProvider>` still
 * render readable strings instead of crashing).
 */
export function useTranslation(): I18nContextValue {
  return useContext(I18nContext);
}

export type { LocaleDict };
