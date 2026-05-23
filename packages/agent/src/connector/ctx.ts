// ConnectorCtx implementation backing Phase 1's in-process model.
//
// Holds a mutable access_token + expiry per connector, refreshes via
// the stdio bridge when near expiry or on 401. Strips any caller-
// provided `Authorization` header so a misbehaving connector can't
// forge another user's identity.

import { log } from "../log.js";
import type {
  ConnectorCtx,
  ConnectorRefreshBridge,
} from "./types.js";

/**
 * Refresh leeway — if the cached expiry is within this window we
 * pre-emptively refresh BEFORE the fetch. Mirrors the Rust-side
 * `REFRESH_LEEWAY` (60 s) so both sides behave consistently.
 */
const REFRESH_LEEWAY_MS = 60_000;

interface MutableTokenState {
  accessToken: string;
  expiresAt: string | null;
}

export interface CreateCtxParams {
  connectorId: string;
  accountEmail: string | null;
  grantedScopes: string[];
  accessToken: string;
  expiresAt: string | null;
  bridge: ConnectorRefreshBridge;
}

export function createCtx(params: CreateCtxParams): ConnectorCtx {
  const state: MutableTokenState = {
    accessToken: params.accessToken,
    expiresAt: params.expiresAt,
  };

  async function refreshAndApply(): Promise<void> {
    const fresh = await params.bridge.refresh(params.connectorId);
    state.accessToken = fresh.accessToken;
    state.expiresAt = fresh.expiresAt;
  }

  return {
    connectorId: params.connectorId,
    accountEmail: params.accountEmail,
    grantedScopes: [...params.grantedScopes],

    async fetch(url, init) {
      if (nearExpiry(state.expiresAt)) {
        try {
          await refreshAndApply();
        } catch (err) {
          log.warn("connector.fetch.preempt_refresh_failed", {
            connector_id: params.connectorId,
            error: (err as Error).message,
          });
          // Fall through — maybe the cached token still has a few
          // seconds left and the request will succeed; if not we'll
          // catch 401 below.
        }
      }

      let response = await doFetch(url, init, state.accessToken);

      if (response.status === 401) {
        // One-shot reactive refresh + retry.
        log.info("connector.fetch.401_refresh_retry", {
          connector_id: params.connectorId,
          url: redactUrl(url),
        });
        try {
          await refreshAndApply();
        } catch (err) {
          log.error("connector.fetch.refresh_failed", {
            connector_id: params.connectorId,
            error: (err as Error).message,
          });
          return response; // surface the original 401 to the caller
        }
        response = await doFetch(url, init, state.accessToken);
      }

      return response;
    },

    log(level, msg, meta) {
      log[level](`connector.${params.connectorId}.${msg}`, meta);
    },
  };
}

function nearExpiry(expiresAt: string | null): boolean {
  if (!expiresAt) return false; // no info — assume current
  const t = Date.parse(expiresAt);
  if (Number.isNaN(t)) return false;
  return Date.now() + REFRESH_LEEWAY_MS >= t;
}

async function doFetch(
  url: string,
  init: RequestInit | undefined,
  accessToken: string,
): Promise<Response> {
  const headers = new Headers(init?.headers ?? {});
  // Force-set; ignore any Authorization the connector tried to provide.
  // (`set` overwrites — caller-injected values silently lose.)
  headers.set("Authorization", `Bearer ${accessToken}`);
  return fetch(url, { ...init, headers });
}

/** Strip query string before logging — OAuth `access_token` shouldn't
 *  appear in URLs by spec, but Google sometimes redirects through them
 *  for legacy endpoints. */
function redactUrl(url: string): string {
  const idx = url.indexOf("?");
  return idx >= 0 ? `${url.slice(0, idx)}?...` : url;
}
