// Skill 市场前端调用层。
//
// 都是对 Rust 端 commands::market::* 的薄包装。所有真正的网络 / 文件系统
// 操作都在 Rust 侧 (apps/desktop/src-tauri/src/commands/market.rs)，这里
// 只负责 invoke + 类型。
//
// 类型本来应该从 @corivo/shared-types 引用 ts-rs 自动生成的 MarketSkill /
// MarketMeta — 但 Windows 上 typegen 跑 cargo test 会因 `ort` crate 的
// native DLL 而 STATUS_ENTRYPOINT_NOT_FOUND（与本特性无关的环境问题）。
// P1 阶段先在这里手写一份与 Rust 端结构严格对齐的类型；下一次能跑通
// typegen 时 (macOS / 修好的 Windows) 自动覆盖即可。

import { invoke } from "@tauri-apps/api/core";

/** Mirrors `commands::market::MarketSkill` (Rust) — also matches the API
 *  `/v1/market/skills` response item shape. */
export interface MarketSkill {
  slug: string;
  name: string;
  description: string;
  tags: string[];
  kind: string;
  has_scripts: boolean;
  visibility: "public" | "internal";
  commit_sha: string;
}

/** Mirrors `commands::market::MarketMeta` (Rust) — the `.market-meta.json`
 *  sidecar each install drops next to `SKILL.md`. */
export interface MarketMeta {
  slug: string;
  commit_sha: string;
  sha256: string;
  installed_at: string;
}

/**
 * Fetch the catalog of skills visible to the current user.
 *
 * Auth: Rust automatically attaches the live `corivo_session.access_token`
 * from Config when present. Signed-out users get the anonymous view
 * (public skills only). Token never leaves the Rust process.
 */
export async function marketList(): Promise<MarketSkill[]> {
  return invoke<MarketSkill[]>("skill_market_list");
}

/**
 * Download + verify + install one skill into `~/.corivo/skills/market/<slug>/`.
 * Returns the local meta record (slug, commit_sha, sha256, installed_at).
 *
 * Rust verifies the sha256 against the API's `X-Content-SHA256` header
 * before writing anything to disk; auth is attached the same way as
 * `marketList`.
 */
export async function marketInstall(slug: string): Promise<MarketMeta> {
  return invoke<MarketMeta>("skill_market_install", { slug });
}

/** Remove the install dir for one slug. No-op if not installed. */
export async function marketUninstall(slug: string): Promise<void> {
  await invoke<void>("skill_market_uninstall", { slug });
}

/**
 * List skills currently installed on disk by reading each install dir's
 * `.market-meta.json`. UI uses this to render "已安装 / 可更新" state
 * by diffing the local commit_sha against the catalog's commit_sha.
 */
export async function marketInstalled(): Promise<MarketMeta[]> {
  return invoke<MarketMeta[]>("skill_market_installed");
}
