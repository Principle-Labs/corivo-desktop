import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import type { AuthStatus } from "@/lib/types"

// Sidebar UserCard reads from this store; the source of truth is the
// `AuthStatus` snapshot the boot guard fetches from the Tauri side
// (`auth_status` -> cloud auth status -> cached cloud creds).
// `applyAuthStatus` is called once at boot and again on every login /
// refresh / logout; the store stays a single read for any component
// that just wants display fields.

export type UserProfile = {
  /** Empty when unknown — the UserCard renders the localized
   *  `t.user.defaultName` fallback instead of a hard-coded literal so
   *  switching the UI language updates the placeholder. */
  name: string
  email: string | null
  avatarUrl: string | null
}

const DEFAULT_PROFILE: UserProfile = {
  name: "",
  email: null,
  avatarUrl: null,
}

const userProfileStore = createStore<UserProfile>(() => ({ ...DEFAULT_PROFILE }))

/// Sync the store from a fresh `AuthStatus`. Pass `null` to reset
/// (called on logout). Callers: app-boot.tsx (boot snapshot),
/// login-page.tsx (post-login), and any settings UI that touches
/// auth state.
export function applyAuthStatus(status: AuthStatus | null): void {
  if (!status || !status.loggedIn) {
    userProfileStore.setState(DEFAULT_PROFILE)
    return
  }
  userProfileStore.setState({
    // Prefer the Google display name; fall back to email local-part
    // so the sidebar never shows the localized default once we
    // know who the user is. Leaving this empty triggers the i18n
    // fallback in UserCard.
    name: status.name ?? status.email?.split("@")[0] ?? "",
    email: status.email,
    avatarUrl: status.avatarUrl,
  })
}

export function useUserProfile(): UserProfile {
  return useStore(userProfileStore)
}
