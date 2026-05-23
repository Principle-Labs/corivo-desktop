import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "@/i18n";
import { getConfig, setConfig } from "@/lib/tauri";
import type { Config } from "@/lib/types";

const CONFIG_QUERY_KEY = ["config"] as const;
/// Cross-window broadcast emitted by `set_config` (and the
/// `save_anthropic_api_key` / `delete_anthropic_api_key` shortcuts).
/// Mirrors `ConfigChanged::EVENT` in
/// `apps/desktop/src-tauri/src/domain/config.rs`. Without this listener
/// the Quick Ask overlay's React Query cache stays stale after the user
/// flips a setting in the main window, so e.g. a UI-language change
/// wouldn't reach the overlay until it was reloaded.
const CONFIG_CHANGED_EVENT = "config:changed";

/**
 * Mount once per webview at the React root so `["config"]` consumers in
 * that window stay in sync with mutations from any other window. Both
 * the main window (via AppBoot) and the Quick Ask overlay (via its own
 * QuickAskWindow root) call this.
 */
export function useConfigSync() {
  const qc = useQueryClient();
  useEffect(() => {
    const unlistenPromise = listen(CONFIG_CHANGED_EVENT, () => {
      void qc.invalidateQueries({ queryKey: CONFIG_QUERY_KEY });
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [qc]);
}

export function useConfig() {
  const queryClient = useQueryClient();
  // useConfig itself is the source of `Config.app.ui_language` for the
  // I18nProvider, so the hook can't depend on the provider being mounted
  // *inside* it. The translation only kicks in for the toast message,
  // which fires after a successful round-trip — by then the provider is
  // up. If the hook is used before the provider mounts (e.g. in tests),
  // useTranslation falls back to the Chinese dictionary.
  const { t } = useTranslation();

  const query = useQuery({
    queryKey: CONFIG_QUERY_KEY,
    queryFn: getConfig,
    staleTime: Infinity,
  });

  const mutation = useMutation({
    mutationFn: (config: Config) => setConfig(config),
    onSuccess: () => {
      // The backend broadcasts `config:changed` on its own write path,
      // which both windows pick up via `useConfigSync`. Invalidate
      // locally too so the UI updates without waiting for the
      // round-trip.
      void queryClient.invalidateQueries({ queryKey: CONFIG_QUERY_KEY });
    },
    onError: (error: unknown) => {
      toast.error(t.common.saveFailed(String(error)));
    },
  });

  const update = (updater: (prev: Config) => Config) => {
    if (!query.data) {
      return;
    }
    mutation.mutate(updater(query.data));
  };

  return {
    config: query.data,
    isLoading: query.isLoading,
    update,
    isSaving: mutation.isPending,
  };
}
