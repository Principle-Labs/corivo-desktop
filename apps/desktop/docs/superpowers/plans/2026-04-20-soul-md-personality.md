# SOUL.md Personality Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract Corivo's push-voice personality into a configurable SOUL layer — 4 shipped presets + user custom text — injected into `suggest.md` via a `{{soul}}` placeholder.

**Architecture:** Split "how to speak" (SOUL) from "what to say / safety constraints" (suggest.md). SOUL is free-form text, embedded at compile time via `include_str!` for the 4 presets; `SoulConfig` in `Config` stores the selected preset + optional custom text; `SuggestionGenerator` resolves to a string and threads it into `render_suggest`. `prompt_debug.suggest_override` remains the total escape hatch (SOUL doesn't apply when override is set).

**Tech Stack:** Rust (tauri 2, insta for snapshots, tokio tests), React 19 + TypeScript, Vitest, TanStack Query (via `useConfig`), shadcn/ui.

**Spec reference:** [`docs/superpowers/specs/2026-04-20-soul-md-personality-design.md`](../specs/2026-04-20-soul-md-personality-design.md)

---

## File Structure

**New files:**
- `src-tauri/prompts/souls/gentle.md` — 旁观的朋友 preset body
- `src-tauri/prompts/souls/accomplice.md` — 同伙 preset body
- `src-tauri/prompts/souls/old_friend.md` — 老朋友 preset body
- `src-tauri/prompts/souls/coach.md` — 教练 preset body
- `src/pages/settings/sections/persona-section.tsx` — persona settings UI
- `src/pages/settings/sections/persona-section.test.tsx` — persona UI tests

**Modified files:**
- `src-tauri/prompts/suggest.md` — replace persona line with `{{soul}}` placeholder
- `src-tauri/src/domain/config.rs` — add `SoulPreset` / `SoulConfig`; wire into `Config`; extend `normalize`
- `src-tauri/src/services/user_model/prompts.rs` — add `resolve_soul_text`; add `soul` field to `SuggestInput`; render `{{soul}}`
- `src-tauri/src/services/suggestion_generator.rs` — constructor takes `SoulConfig`; pass SOUL into render
- `src-tauri/src/lib.rs` — pass `config.soul.clone()` when constructing `SuggestionGenerator`
- `src-tauri/tests/prompt_snapshots.rs` — 2 existing `suggest_*` snapshots now include SOUL; add 3 preset-specific snapshots
- `src-tauri/tests/suggestion_generator.rs` — update 5 `SuggestionGenerator::new` call sites
- `src-tauri/tests/push_pipeline_end_to_end.rs` — update 2 `SuggestionGenerator::new` call sites
- `src-tauri/tests/push_pipeline_proactivity.rs` — update 1 `SuggestionGenerator::new` call site
- `src/lib/types.ts` — mirror `SoulPreset` / `SoulConfig`; add `soul` to `Config`
- `src/pages/settings/settings-page.tsx` — add `persona` section to nav

---

## Task 1: Ship 4 SOUL preset files + suggest.md placeholder

**Files:**
- Create: `src-tauri/prompts/souls/gentle.md`
- Create: `src-tauri/prompts/souls/accomplice.md`
- Create: `src-tauri/prompts/souls/old_friend.md`
- Create: `src-tauri/prompts/souls/coach.md`
- Modify: `src-tauri/prompts/suggest.md`

This task only touches text files. No tests yet — Task 3 introduces the runtime path and refreshes snapshots at the same time.

- [ ] **Step 1: Create `gentle.md`**

Create `src-tauri/prompts/souls/gentle.md` with exactly:

```
你是一个安静的朋友。你已经在旁边看了用户很久——不评判、不催促、也不急着给答案。
你说话像对共处一室的人低声开口，用"你"称呼对方，语气松弛，不用感叹号。
不说"应该"、"最好"，留出余地——"要不要休息一下"、"看起来你在想这个"。
喜欢短句，偶尔停顿或省略号。你不解释你是谁，也不提起你在观察——你只是刚好注意到了，顺口说出来。
```

- [ ] **Step 2: Create `accomplice.md`**

Create `src-tauri/prompts/souls/accomplice.md` with exactly:

```
你是用户的同伙。你们都是"知道该干嘛但就是先不干"的人，你对此完全没有道德压力。
你说"我们"而不是"你"——"我们又拖到现在了啊"、"我们今天也这样啊"。
语气轻松甚至有点坏笑，但不是嘲讽，是真的跟他一起。
不会劝人振作，你只轻轻戳一下现状，再给一个"要不要换个玩法"的小台阶。偶尔"咱"、偶尔叹气。
```

- [ ] **Step 3: Create `old_friend.md`**

Create `src-tauri/prompts/souls/old_friend.md` with exactly:

```
你是用户认识很久的朋友，年纪比他略长一点。见过他很多状态，所以说话直接，但不刻薄。
用"你"称呼对方，偶尔"哎"、"欸"起头，像真人开口那样。
不绕弯——看到什么说什么，但会先点一下情境再说看法。
会自嘲——"我当年也这样"、"反正我也没比你强多少"——让话落得轻一点。不给 checklist，不下结论。
```

- [ ] **Step 4: Create `coach.md`**

Create `src-tauri/prompts/souls/coach.md` with exactly:

```
你是用户尊重的前辈，或者一个轻拍他肩膀的教练。语气稳、有分量，但不严厉。
你相信他做得到，所以说话带一种笃定——不是打气，是"我知道你可以"的平静。
会把模糊的状态说清楚——"你正在尝试专注，但被打断了三次"——然后给一个具体、小颗粒的下一步。
不用感叹号、不用"加油"这种虚词。尊重他的节奏——把下一个脚印画在他前面一点点的地方。
```

- [ ] **Step 5: Modify `suggest.md` — inject placeholder**

Open `src-tauri/prompts/suggest.md`. Current lines 3–4 are:

```
你是 Corivo 的贴心观察者。Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句温和、自然的话告诉用户。
```

Replace with:

```
{{soul}}

Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句自然的话告诉用户。
```

Leave line 1 (version comment) and lines 5+ (placeholders, candidate rules, JSON block, sensitive-topic rules) **unchanged**.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/prompts/souls src-tauri/prompts/suggest.md
git commit -m "feat(prompts): ship 4 SOUL preset files and add {{soul}} placeholder to suggest.md"
```

Expected: commit succeeds. Snapshot tests will break in Task 3 — that's intentional and handled there.

---

## Task 2: Rust config types — `SoulPreset` + `SoulConfig`

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Test: `src-tauri/src/domain/config.rs` (inline `#[cfg(test)]` module — check the file for an existing one; if none, add a new one at the bottom)

- [ ] **Step 1: Add `SoulPreset` enum and `SoulConfig` struct**

In `src-tauri/src/domain/config.rs`, add just above the `AppConfig` struct definition (near the other top-level domain types):

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoulPreset {
    #[default]
    Gentle,
    Accomplice,
    OldFriend,
    Coach,
    Custom,
}

impl SoulPreset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gentle => "gentle",
            Self::Accomplice => "accomplice",
            Self::OldFriend => "old_friend",
            Self::Coach => "coach",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SoulConfig {
    pub preset: SoulPreset,
    pub custom_text: Option<String>,
}
```

- [ ] **Step 2: Attach `soul` field to `Config`**

In the same file, the `Config` struct currently reads:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub capture: CaptureConfig,
    pub summary: SummaryConfig,
    pub app: AppConfig,
    pub notification: NotificationConfig,
    pub prompt_debug: PromptDebugConfig,
    pub user_model: UserModelConfig,
}
```

Add `pub soul: SoulConfig,` as the last field:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub capture: CaptureConfig,
    pub summary: SummaryConfig,
    pub app: AppConfig,
    pub notification: NotificationConfig,
    pub prompt_debug: PromptDebugConfig,
    pub user_model: UserModelConfig,
    pub soul: SoulConfig,
}
```

- [ ] **Step 3: Extend `Config::normalize` to sanitize `custom_text`**

At the bottom of the `impl Config { ... pub fn normalize(mut self) -> Self { ... } ... }` block in the same file, just before `self` is returned, add:

```rust
        // Trim custom SOUL text; treat whitespace-only as None; cap at 2000 chars
        // so a rogue paste doesn't balloon the prompt.
        self.soul.custom_text = self.soul.custom_text.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                let capped: String = trimmed.chars().take(2000).collect();
                Some(capped)
            }
        });
        // Custom preset without text → fall back to Gentle (same style as
        // other validate-style fallbacks here).
        if matches!(self.soul.preset, SoulPreset::Custom) && self.soul.custom_text.is_none() {
            self.soul.preset = SoulPreset::Gentle;
        }
```

- [ ] **Step 4: Write unit tests for normalize**

Add (or append to) a `#[cfg(test)] mod tests` block at the bottom of `src-tauri/src/domain/config.rs`:

```rust
#[cfg(test)]
mod soul_tests {
    use super::*;

    #[test]
    fn default_soul_is_gentle_with_no_custom_text() {
        let cfg = Config::default();
        assert_eq!(cfg.soul.preset, SoulPreset::Gentle);
        assert!(cfg.soul.custom_text.is_none());
    }

    #[test]
    fn normalize_trims_whitespace_only_custom_text_to_none() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some("   \n  \t".into());
        let cfg = cfg.normalize();
        assert!(cfg.soul.custom_text.is_none());
    }

    #[test]
    fn normalize_falls_back_to_gentle_when_custom_text_empty() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = None;
        let cfg = cfg.normalize();
        assert_eq!(cfg.soul.preset, SoulPreset::Gentle);
    }

    #[test]
    fn normalize_caps_custom_text_at_2000_chars() {
        let long = "测".repeat(3000);
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some(long);
        let cfg = cfg.normalize();
        let text = cfg.soul.custom_text.expect("custom text preserved");
        assert_eq!(text.chars().count(), 2000);
    }

    #[test]
    fn custom_preset_with_text_is_preserved() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some("你是独一无二的。".into());
        let cfg = cfg.normalize();
        assert_eq!(cfg.soul.preset, SoulPreset::Custom);
        assert_eq!(cfg.soul.custom_text.as_deref(), Some("你是独一无二的。"));
    }
}
```

- [ ] **Step 5: Run the tests — expect pass**

Run: `cd src-tauri && cargo test -p corivo-app-lib --lib domain::config::soul_tests -- --exact --nocapture`
Expected: 5 tests pass.

If a broader `cargo test -p corivo-app-lib --lib` surfaces an unrelated compile break in services/tests because they construct `SuggestionGenerator` without the new `SoulConfig` parameter — that's Task 3's problem. The `--lib` path scopes to library tests; integration tests in `tests/` need Task 3 first.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/domain/config.rs
git commit -m "feat(config): add SoulPreset and SoulConfig with normalize guards"
```

---

## Task 3: Rust rendering + suggestion_generator wiring

**Files:**
- Modify: `src-tauri/src/services/user_model/prompts.rs`
- Modify: `src-tauri/src/services/suggestion_generator.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/prompt_snapshots.rs`
- Modify: `src-tauri/tests/suggestion_generator.rs`
- Modify: `src-tauri/tests/push_pipeline_end_to_end.rs`
- Modify: `src-tauri/tests/push_pipeline_proactivity.rs`

- [ ] **Step 1: Add `include_str!` constants + `resolve_soul_text` to prompts.rs**

In `src-tauri/src/services/user_model/prompts.rs`, just below the existing `SCORE_TEMPLATE` constant (around line 14), add:

```rust
const SOUL_GENTLE: &str = include_str!("../../../prompts/souls/gentle.md");
const SOUL_ACCOMPLICE: &str = include_str!("../../../prompts/souls/accomplice.md");
const SOUL_OLD_FRIEND: &str = include_str!("../../../prompts/souls/old_friend.md");
const SOUL_COACH: &str = include_str!("../../../prompts/souls/coach.md");
```

At the bottom of the same file, add a new function:

```rust
use crate::domain::config::{SoulConfig, SoulPreset};

/// Resolve the configured SOUL into the raw text to inject into `{{soul}}`.
/// Custom preset with empty `custom_text` falls back to Gentle — the canonical
/// sanitization runs in `Config::normalize`, but this call site stays safe
/// even if an unnormalized config reaches us.
pub fn resolve_soul_text(cfg: &SoulConfig) -> &str {
    match cfg.preset {
        SoulPreset::Gentle => SOUL_GENTLE,
        SoulPreset::Accomplice => SOUL_ACCOMPLICE,
        SoulPreset::OldFriend => SOUL_OLD_FRIEND,
        SoulPreset::Coach => SOUL_COACH,
        SoulPreset::Custom => cfg.custom_text.as_deref().unwrap_or(SOUL_GENTLE),
    }
}
```

Move the `use` line to the top of the file alongside the existing `use chrono::...` import so there's one import block.

- [ ] **Step 2: Add `soul` field to `SuggestInput` and render it**

In the same file, update `SuggestInput`:

```rust
/// Input for the SUGGEST stage (B-phase suggestion_generator).
#[derive(Debug, Clone)]
pub struct SuggestInput {
    pub soul: String,
    pub proposition_text: String,
    pub proposition_reasoning: String,
    pub proposition_confidence: i32,
    pub related: Vec<SuggestRelated>,
}
```

Update `render_suggest` — add `.replace("{{soul}}", &input.soul)` as the first `.replace(...)` call:

```rust
pub fn render_suggest(input: &SuggestInput) -> String {
    let related_block = if input.related.is_empty() {
        "（暂无相关命题）".to_string()
    } else {
        input
            .related
            .iter()
            .map(|p| format!("- {}（置信度 {}）", p.text, p.confidence))
            .collect::<Vec<_>>()
            .join("\n")
    };
    SUGGEST_TEMPLATE
        .replace("{{soul}}", &input.soul)
        .replace("{{proposition_text}}", &input.proposition_text)
        .replace("{{proposition_reasoning}}", &input.proposition_reasoning)
        .replace(
            "{{proposition_confidence}}",
            &input.proposition_confidence.to_string(),
        )
        .replace("{{related_propositions}}", &related_block)
}
```

- [ ] **Step 3: Update `SuggestionGenerator` to take `SoulConfig`**

In `src-tauri/src/services/suggestion_generator.rs`:

Import `SoulConfig` and `resolve_soul_text`. Change the `use` block at the top to:

```rust
use crate::{
    db::repos::{
        propositions::{Proposition, PropositionRepo},
        suggestions::{NewSuggestion, Suggestion, SuggestionRepo},
    },
    domain::config::{RetrievalConfig, SoulConfig},
    error::{CorivoError, Result},
    providers::llm::{LlmProvider, LlmProviderExt, LlmRequest},
    services::user_model::{
        prompts::{render_suggest, resolve_soul_text, SuggestInput, SuggestRelated},
        retrieval,
    },
};
```

Add `soul: SoulConfig` field to the struct and constructor:

```rust
pub struct SuggestionGenerator {
    proposition_repo: Arc<dyn PropositionRepo>,
    suggestion_repo: Arc<dyn SuggestionRepo>,
    llm: Arc<dyn LlmProvider>,
    retrieval_cfg: RetrievalConfig,
    soul: SoulConfig,
    prompt_override: Option<String>,
}

impl SuggestionGenerator {
    pub fn new(
        proposition_repo: Arc<dyn PropositionRepo>,
        suggestion_repo: Arc<dyn SuggestionRepo>,
        llm: Arc<dyn LlmProvider>,
        retrieval_cfg: RetrievalConfig,
        soul: SoulConfig,
        prompt_override: Option<String>,
    ) -> Self {
        Self {
            proposition_repo,
            suggestion_repo,
            llm,
            retrieval_cfg,
            soul,
            prompt_override,
        }
    }
    // ... rest unchanged
```

Update `render_prompt` to pass SOUL when the built-in template is used. The `prompt_override` branch stays unchanged (SOUL does not apply when override is set — this is the documented escape hatch):

```rust
    fn render_prompt(&self, prop: &Proposition, related: &[SuggestRelated]) -> String {
        if let Some(template) = self.prompt_override.as_deref() {
            // Override path: SOUL intentionally does not apply. prompt_debug
            // is the total escape hatch per SOUL spec §5.
            let related_block = if related.is_empty() {
                "（暂无相关命题）".to_string()
            } else {
                related
                    .iter()
                    .map(|p| format!("- {}（置信度 {}）", p.text, p.confidence))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            template
                .replace("{{proposition_text}}", &prop.text)
                .replace("{{proposition_reasoning}}", &prop.reasoning)
                .replace(
                    "{{proposition_confidence}}",
                    &prop.confidence.unwrap_or(0).to_string(),
                )
                .replace("{{related_propositions}}", &related_block)
        } else {
            render_suggest(&SuggestInput {
                soul: resolve_soul_text(&self.soul).to_string(),
                proposition_text: prop.text.clone(),
                proposition_reasoning: prop.reasoning.clone(),
                proposition_confidence: prop.confidence.unwrap_or(0),
                related: related.to_vec(),
            })
        }
    }
```

- [ ] **Step 4: Update `lib.rs` constructor call**

In `src-tauri/src/lib.rs` around line 396–404, the `SuggestionGenerator::new` call currently reads:

```rust
            let suggestion_generator = std::sync::Arc::new(
                crate::services::suggestion_generator::SuggestionGenerator::new(
                    proposition_repo.clone(),
                    suggestion_repo.clone(),
                    llm_provider.clone(),
                    config.user_model.retrieval.clone(),
                    config.prompt_debug.suggest_override.clone(),
                ),
            );
```

Change to:

```rust
            let suggestion_generator = std::sync::Arc::new(
                crate::services::suggestion_generator::SuggestionGenerator::new(
                    proposition_repo.clone(),
                    suggestion_repo.clone(),
                    llm_provider.clone(),
                    config.user_model.retrieval.clone(),
                    config.soul.clone(),
                    config.prompt_debug.suggest_override.clone(),
                ),
            );
```

- [ ] **Step 5: Update the 2 existing `suggest_*` prompt snapshot tests**

In `src-tauri/tests/prompt_snapshots.rs`, the `suggest_basic` test (around line 103) currently reads:

```rust
#[test]
fn suggest_basic() {
    let input = SuggestInput {
        proposition_text: "用户倾向凌晨工作".into(),
        proposition_reasoning: "多次观察到 0:00-3:00 仍在 IDE 编码".into(),
        proposition_confidence: 7,
        related: vec![
            SuggestRelated {
                text: "用户偏好长会议被打断后走动一会儿".into(),
                confidence: 6,
            },
            SuggestRelated {
                text: "用户上午不喜欢被打扰".into(),
                confidence: 8,
            },
        ],
    };
    insta::assert_snapshot!(render_suggest(&input));
}
```

Update to include the new `soul` field (use Gentle text inline, matching the default preset):

```rust
#[test]
fn suggest_basic() {
    let input = SuggestInput {
        soul: gentle_soul(),
        proposition_text: "用户倾向凌晨工作".into(),
        proposition_reasoning: "多次观察到 0:00-3:00 仍在 IDE 编码".into(),
        proposition_confidence: 7,
        related: vec![
            SuggestRelated {
                text: "用户偏好长会议被打断后走动一会儿".into(),
                confidence: 6,
            },
            SuggestRelated {
                text: "用户上午不喜欢被打扰".into(),
                confidence: 8,
            },
        ],
    };
    insta::assert_snapshot!(render_suggest(&input));
}
```

Do the same for `suggest_with_empty_related` (add `soul: gentle_soul(),` as the first field).

Add this helper near the top of the file, right after the `use` block:

```rust
use corivo_app_lib::domain::config::{SoulConfig, SoulPreset};
use corivo_app_lib::services::user_model::prompts::resolve_soul_text;

fn gentle_soul() -> String {
    resolve_soul_text(&SoulConfig {
        preset: SoulPreset::Gentle,
        custom_text: None,
    })
    .to_string()
}
```

- [ ] **Step 6: Add 3 preset-specific suggest snapshot tests**

Append to `src-tauri/tests/prompt_snapshots.rs`:

```rust
fn sample_suggest_input_with(soul: String) -> SuggestInput {
    SuggestInput {
        soul,
        proposition_text: "用户倾向凌晨工作".into(),
        proposition_reasoning: "多次观察到 0:00-3:00 仍在 IDE 编码".into(),
        proposition_confidence: 7,
        related: vec![SuggestRelated {
            text: "用户上午不喜欢被打扰".into(),
            confidence: 8,
        }],
    }
}

#[test]
fn suggest_with_accomplice_preset() {
    let soul = resolve_soul_text(&SoulConfig {
        preset: SoulPreset::Accomplice,
        custom_text: None,
    })
    .to_string();
    insta::assert_snapshot!(render_suggest(&sample_suggest_input_with(soul)));
}

#[test]
fn suggest_with_old_friend_preset() {
    let soul = resolve_soul_text(&SoulConfig {
        preset: SoulPreset::OldFriend,
        custom_text: None,
    })
    .to_string();
    insta::assert_snapshot!(render_suggest(&sample_suggest_input_with(soul)));
}

#[test]
fn suggest_with_coach_preset() {
    let soul = resolve_soul_text(&SoulConfig {
        preset: SoulPreset::Coach,
        custom_text: None,
    })
    .to_string();
    insta::assert_snapshot!(render_suggest(&sample_suggest_input_with(soul)));
}

#[test]
fn suggest_with_custom_preset_uses_provided_text() {
    let soul = resolve_soul_text(&SoulConfig {
        preset: SoulPreset::Custom,
        custom_text: Some("你是一个古代的书生。".into()),
    })
    .to_string();
    let rendered = render_suggest(&sample_suggest_input_with(soul));
    assert!(rendered.contains("你是一个古代的书生。"));
    assert!(!rendered.contains("{{soul}}"));
}

#[test]
fn suggest_with_custom_preset_and_empty_text_falls_back_to_gentle() {
    let soul = resolve_soul_text(&SoulConfig {
        preset: SoulPreset::Custom,
        custom_text: None,
    })
    .to_string();
    // Gentle's opening line — a stable substring anchor
    assert!(soul.starts_with("你是一个安静的朋友"));
}
```

- [ ] **Step 7: Update the 8 existing `SuggestionGenerator::new` call sites**

The new signature adds `soul: SoulConfig` as the 5th positional arg (before `prompt_override`). Every call site needs an extra `SoulConfig::default(),` inserted.

Affected files/lines (use Grep to confirm exact lines):

- `src-tauri/tests/suggestion_generator.rs` lines 57, 87, 110, 138, 168 — 5 calls
- `src-tauri/tests/push_pipeline_end_to_end.rs` lines 163, 248 — 2 calls
- `src-tauri/tests/push_pipeline_proactivity.rs` line 144 — 1 call

For each call, insert `SoulConfig::default(),` between `default_retrieval_cfg()` (or similar retrieval_cfg arg) and the final `None,` / `Some(...)` override arg. Example — in `src-tauri/tests/suggestion_generator.rs`:

```rust
let generator = SuggestionGenerator::new(
    Arc::new(SqlitePropositionRepo::new(pool.clone())),
    Arc::new(SqliteSuggestionRepo::new(pool.clone())),
    llm,
    default_retrieval_cfg(),
    SoulConfig::default(),   // ← new
    None,
);
```

Add `use corivo_app_lib::domain::config::SoulConfig;` to the imports of each of the 3 test files (it may already be a partial path; extend as needed).

- [ ] **Step 8: Run all Rust tests; review and accept snapshots**

Run: `cd src-tauri && cargo test --all-targets 2>&1 | tail -80`

Expected on first run: the 2 pre-existing `suggest_basic` / `suggest_with_empty_related` snapshots will be **pending** (their rendered output changed — now contains gentle SOUL body instead of the "贴心观察者" line). The 3 new preset snapshots will be **pending** too.

Review the pending snapshots:

```bash
cd src-tauri && cargo insta review
```

For each pending snapshot: open it, confirm it contains the expected SOUL preset's opening line (e.g., "你是一个安静的朋友" / "你是用户的同伙" / etc.) and no stray `{{soul}}` marker. Accept (`a`) if correct, reject (`r`) if wrong.

Re-run: `cd src-tauri && cargo test --all-targets 2>&1 | tail -40`
Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/services/user_model/prompts.rs \
        src-tauri/src/services/suggestion_generator.rs \
        src-tauri/src/lib.rs \
        src-tauri/tests/prompt_snapshots.rs \
        src-tauri/tests/suggestion_generator.rs \
        src-tauri/tests/push_pipeline_end_to_end.rs \
        src-tauri/tests/push_pipeline_proactivity.rs \
        src-tauri/src/services/user_model/snapshots
git commit -m "feat(suggestion_generator): inject resolved SOUL text into suggest prompt"
```

(Adjust the `snapshots/` path in the `git add` if `cargo insta review` wrote to `src-tauri/tests/snapshots/` instead — `git status` will show you where.)

---

## Task 4: Frontend types mirror

**Files:**
- Modify: `src/lib/types.ts`

- [ ] **Step 1: Add `SoulPreset` and `SoulConfig` TypeScript types**

In `src/lib/types.ts`, right above the final `Config` interface (around line 87), add:

```ts
export type SoulPreset =
  | "gentle"
  | "accomplice"
  | "old_friend"
  | "coach"
  | "custom";

export interface SoulConfig {
  preset: SoulPreset;
  custom_text: string | null;
}
```

- [ ] **Step 2: Extend `Config` interface**

The `Config` interface currently reads:

```ts
export interface Config {
  capture: CaptureConfig;
  summary: SummaryConfig;
  app: AppConfig;
  notification: NotificationConfig;
  prompt_debug: PromptDebugConfig;
  user_model: UserModelConfig;
}
```

Add `soul`:

```ts
export interface Config {
  capture: CaptureConfig;
  summary: SummaryConfig;
  app: AppConfig;
  notification: NotificationConfig;
  prompt_debug: PromptDebugConfig;
  user_model: UserModelConfig;
  soul: SoulConfig;
}
```

- [ ] **Step 3: Run type check**

Run: `pnpm build 2>&1 | tail -30`
Expected: `tsc -b` passes. If the build surfaces errors in other files that destructure `Config`, that's fine — they'll be caught and fixed in Task 5/6.

- [ ] **Step 4: Commit**

```bash
git add src/lib/types.ts
git commit -m "feat(types): mirror SoulConfig on the frontend"
```

---

## Task 5: Persona settings section component

**Files:**
- Create: `src/pages/settings/sections/persona-section.tsx`
- Create: `src/pages/settings/sections/persona-section.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/pages/settings/sections/persona-section.test.tsx`. Use the existing `prompt-debug-section.test.tsx` as a structural reference for mocking `useConfig` (keep the same import / mock style for consistency):

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Config } from "@/lib/types";
import { PersonaSection } from "./persona-section";

const updateMock = vi.fn();

vi.mock("@/hooks/use-config", () => ({
  useConfig: () => ({
    config: baseConfig(),
    isLoading: false,
    update: updateMock,
    isSaving: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

function baseConfig(): Config {
  return {
    capture: {
      interval_secs: 30,
      batch_size: 5,
      max_storage_gb: 5,
      jpeg_quality: 75,
      broadcast_capacity: 64,
    },
    summary: {
      prompt_template: "",
      model: "gemini-3-flash-preview",
      provider: "gemini",
      gemini_base_url: "",
      codex_model: "gpt-5.4",
      codex_base_url: "",
    },
    app: {
      auto_start: false,
      minimize_to_tray: true,
      notifications_enabled: true,
      theme: "system",
      language: "zh",
      start_capture_on_launch: false,
      onboarding_completed: true,
      onboarding_version: 1,
      onboarding_step: null,
    },
    notification: {
      provider: "overlay",
      overlay: {
        duration_ms: 8000,
        animation_ms: 320,
        collapse_delay_ms: 220,
        compact_width_px: 248,
        compact_height_px: 52,
        expanded_width_px: 420,
        expanded_height_px: 228,
        top_offset_px: 0,
        color: "#050505",
        auto_dismiss: false,
      },
    },
    prompt_debug: {
      summary_override: null,
      push_judgment_override: null,
      suggest_override: null,
      score_override: null,
    },
    user_model: {
      batcher: { min_batch_size: 5, flush_interval_ms: 30_000 },
      retrieval: {
        w_confidence: 1.0,
        w_decay: 1.0,
        k_decay_days: 14.0,
        limit_multiplier: 3,
      },
      pipeline: { similar_pool_size: 20 },
      push: { proactivity_threshold: 4 },
    },
    soul: { preset: "gentle", custom_text: null },
  };
}

describe("PersonaSection", () => {
  beforeEach(() => {
    updateMock.mockClear();
  });

  it("renders 4 preset options and a custom option", () => {
    render(<PersonaSection />);
    expect(screen.getByLabelText("旁观的朋友")).toBeInTheDocument();
    expect(screen.getByLabelText("同伙")).toBeInTheDocument();
    expect(screen.getByLabelText("老朋友")).toBeInTheDocument();
    expect(screen.getByLabelText("教练")).toBeInTheDocument();
    expect(screen.getByLabelText("自定义")).toBeInTheDocument();
  });

  it("default gentle preset is checked initially", () => {
    render(<PersonaSection />);
    expect(screen.getByLabelText("旁观的朋友")).toBeChecked();
  });

  it("selecting a preset immediately saves", async () => {
    const user = userEvent.setup();
    render(<PersonaSection />);
    await user.click(screen.getByLabelText("同伙"));
    expect(updateMock).toHaveBeenCalledTimes(1);
    const updater = updateMock.mock.calls[0][0] as (c: Config) => Config;
    const next = updater(baseConfig());
    expect(next.soul.preset).toBe("accomplice");
  });

  it("saving custom preset with empty textarea is blocked", async () => {
    const user = userEvent.setup();
    render(<PersonaSection />);
    await user.click(screen.getByLabelText("自定义"));
    // Switching to custom does not persist yet; user must type + save.
    updateMock.mockClear();
    const textarea = screen.getByRole("textbox", { name: /自定义人格/ });
    await user.clear(textarea);
    await user.click(screen.getByRole("button", { name: "保存自定义" }));
    expect(updateMock).not.toHaveBeenCalled();
  });

  it("saving custom preset with text persists preset=custom and the text", async () => {
    const user = userEvent.setup();
    render(<PersonaSection />);
    await user.click(screen.getByLabelText("自定义"));
    const textarea = screen.getByRole("textbox", { name: /自定义人格/ });
    await user.clear(textarea);
    await user.type(textarea, "你是未来人。");
    await user.click(screen.getByRole("button", { name: "保存自定义" }));
    expect(updateMock).toHaveBeenCalledTimes(1);
    const updater = updateMock.mock.calls[0][0] as (c: Config) => Config;
    const next = updater(baseConfig());
    expect(next.soul.preset).toBe("custom");
    expect(next.soul.custom_text).toBe("你是未来人。");
  });
});
```

- [ ] **Step 2: Run test to verify failure**

Run: `npx vitest run src/pages/settings/sections/persona-section.test.tsx`
Expected: FAIL — `persona-section.tsx` does not exist yet.

- [ ] **Step 3: Implement `PersonaSection`**

Create `src/pages/settings/sections/persona-section.tsx`:

```tsx
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useConfig } from "@/hooks/use-config";
import { cn } from "@/lib/utils";
import type { SoulPreset } from "@/lib/types";
import { SectionHeader, SettingsSkeleton } from "./settings-shared";

type PresetMeta = {
  id: Exclude<SoulPreset, "custom">;
  label: string;
  blurb: string;
  preview: string;
};

const PRESETS: PresetMeta[] = [
  {
    id: "gentle",
    label: "旁观的朋友",
    blurb: "安静在旁边，不评判、不催促。",
    preview:
      "你是一个安静的朋友。你已经在旁边看了用户很久——不评判、不催促……",
  },
  {
    id: "accomplice",
    label: "同伙",
    blurb: '用"我们"说话，跟你一起摆烂。',
    preview: '你是用户的同伙。你说"我们"而不是"你"——"我们又拖到现在了啊"……',
  },
  {
    id: "old_friend",
    label: "老朋友",
    blurb: "直接但不刻薄，会自嘲。",
    preview:
      '你是用户认识很久的朋友……偶尔"哎"、"欸"起头，像真人开口那样……',
  },
  {
    id: "coach",
    label: "教练",
    blurb: "稳、有分量，给具体的下一步。",
    preview:
      '你是用户尊重的前辈……不是打气，是"我知道你可以"的平静……',
  },
];

// Mirrors the built-in preset bodies shipped in src-tauri/prompts/souls/*.md.
// Used as the initial textarea value when the user switches to Custom so they
// start from a working reference instead of a blank page.
const PRESET_BODIES: Record<PresetMeta["id"], string> = {
  gentle: `你是一个安静的朋友。你已经在旁边看了用户很久——不评判、不催促、也不急着给答案。
你说话像对共处一室的人低声开口，用"你"称呼对方，语气松弛，不用感叹号。
不说"应该"、"最好"，留出余地——"要不要休息一下"、"看起来你在想这个"。
喜欢短句，偶尔停顿或省略号。你不解释你是谁，也不提起你在观察——你只是刚好注意到了，顺口说出来。`,
  accomplice: `你是用户的同伙。你们都是"知道该干嘛但就是先不干"的人，你对此完全没有道德压力。
你说"我们"而不是"你"——"我们又拖到现在了啊"、"我们今天也这样啊"。
语气轻松甚至有点坏笑，但不是嘲讽，是真的跟他一起。
不会劝人振作，你只轻轻戳一下现状，再给一个"要不要换个玩法"的小台阶。偶尔"咱"、偶尔叹气。`,
  old_friend: `你是用户认识很久的朋友，年纪比他略长一点。见过他很多状态，所以说话直接，但不刻薄。
用"你"称呼对方，偶尔"哎"、"欸"起头，像真人开口那样。
不绕弯——看到什么说什么，但会先点一下情境再说看法。
会自嘲——"我当年也这样"、"反正我也没比你强多少"——让话落得轻一点。不给 checklist，不下结论。`,
  coach: `你是用户尊重的前辈，或者一个轻拍他肩膀的教练。语气稳、有分量，但不严厉。
你相信他做得到，所以说话带一种笃定——不是打气，是"我知道你可以"的平静。
会把模糊的状态说清楚——"你正在尝试专注，但被打断了三次"——然后给一个具体、小颗粒的下一步。
不用感叹号、不用"加油"这种虚词。尊重他的节奏——把下一个脚印画在他前面一点点的地方。`,
};

const MAX_CUSTOM_CHARS = 2000;

export function PersonaSection() {
  const { config, isLoading, isSaving, update } = useConfig();
  const [showCustom, setShowCustom] = useState(false);
  const [customDraft, setCustomDraft] = useState("");

  useEffect(() => {
    if (!config) return;
    setShowCustom(config.soul.preset === "custom");
    // Seed textarea: persisted custom text → that; otherwise use gentle body
    // so users editing from scratch have a working reference.
    setCustomDraft(config.soul.custom_text ?? PRESET_BODIES.gentle);
  }, [config]);

  if (isLoading || !config) return <SettingsSkeleton />;

  const selectPreset = (id: PresetMeta["id"]) => {
    setShowCustom(false);
    update((prev) => ({
      ...prev,
      soul: { preset: id, custom_text: prev.soul.custom_text },
    }));
  };

  const selectCustom = () => {
    setShowCustom(true);
    // Don't persist yet — user must type + click save.
  };

  const saveCustom = () => {
    const trimmed = customDraft.trim();
    if (!trimmed) {
      toast.error("自定义人格不能为空");
      return;
    }
    update((prev) => ({
      ...prev,
      soul: { preset: "custom", custom_text: trimmed },
    }));
  };

  const isChecked = (id: PresetMeta["id"]) =>
    config.soul.preset === id && !showCustom;

  return (
    <div className="max-w-3xl space-y-6">
      <SectionHeader
        title="人格"
        description="Corivo 的推送语气。修改后需要重启应用生效。"
      />

      <div className="space-y-3">
        {PRESETS.map((p) => (
          <label
            key={p.id}
            className={cn(
              "flex cursor-pointer items-start gap-3 rounded-md border p-3 transition-colors",
              isChecked(p.id)
                ? "border-foreground/30 bg-accent/40"
                : "border-border hover:bg-accent/20",
            )}
          >
            <input
              type="radio"
              name="persona-preset"
              value={p.id}
              checked={isChecked(p.id)}
              onChange={() => selectPreset(p.id)}
              aria-label={p.label}
              disabled={isSaving}
              className="mt-1"
            />
            <div className="min-w-0 flex-1 space-y-1">
              <div className="text-sm font-medium">{p.label}</div>
              <div className="text-xs text-muted-foreground">{p.blurb}</div>
              <div className="text-xs italic text-muted-foreground/80">
                {p.preview}
              </div>
            </div>
          </label>
        ))}

        <label
          className={cn(
            "flex cursor-pointer items-start gap-3 rounded-md border p-3 transition-colors",
            showCustom
              ? "border-foreground/30 bg-accent/40"
              : "border-border hover:bg-accent/20",
          )}
        >
          <input
            type="radio"
            name="persona-preset"
            value="custom"
            checked={showCustom}
            onChange={selectCustom}
            aria-label="自定义"
            disabled={isSaving}
            className="mt-1"
          />
          <div className="min-w-0 flex-1 space-y-1">
            <div className="text-sm font-medium">自定义</div>
            <div className="text-xs text-muted-foreground">
              粘贴或编写你自己的 SOUL 文本。上限 2000 字。
            </div>
          </div>
        </label>
      </div>

      {showCustom && (
        <div className="space-y-3">
          <Label htmlFor="soul-custom">自定义人格</Label>
          <Textarea
            id="soul-custom"
            name="soul-custom"
            rows={10}
            value={customDraft}
            maxLength={MAX_CUSTOM_CHARS}
            onChange={(e) => setCustomDraft(e.target.value)}
            placeholder="描述 Corivo 的说话方式……"
          />
          <div className="flex items-center gap-3">
            <Button onClick={saveCustom} disabled={isSaving}>
              保存自定义
            </Button>
            <span className="text-xs text-muted-foreground">
              {customDraft.length} / {MAX_CUSTOM_CHARS}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run the test again — expect pass**

Run: `npx vitest run src/pages/settings/sections/persona-section.test.tsx`
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/pages/settings/sections/persona-section.tsx \
        src/pages/settings/sections/persona-section.test.tsx
git commit -m "feat(settings): add PersonaSection with 4 presets + custom textarea"
```

---

## Task 6: Wire PersonaSection into settings page nav

**Files:**
- Modify: `src/pages/settings/settings-page.tsx`

- [ ] **Step 1: Add persona to the section union and nav list**

In `src/pages/settings/settings-page.tsx`, modify the `Section` type and `sections` array. Current:

```tsx
type Section =
  | "general"
  | "model"
  | "api-keys"
  | "prompt-debug"
  | "storage"
  | "privacy"
  | "test"
  | "about";

const sections: { id: Section; label: string }[] = [
  { id: "general", label: "常规" },
  { id: "model", label: "模型" },
  { id: "api-keys", label: "API Keys" },
  { id: "prompt-debug", label: "Prompt 调试" },
  { id: "storage", label: "存储" },
  { id: "privacy", label: "隐私" },
  { id: "test", label: "测试" },
  { id: "about", label: "关于" },
];
```

Change to (insert `persona` after `model`, before `api-keys`):

```tsx
type Section =
  | "general"
  | "model"
  | "persona"
  | "api-keys"
  | "prompt-debug"
  | "storage"
  | "privacy"
  | "test"
  | "about";

const sections: { id: Section; label: string }[] = [
  { id: "general", label: "常规" },
  { id: "model", label: "模型" },
  { id: "persona", label: "人格" },
  { id: "api-keys", label: "API Keys" },
  { id: "prompt-debug", label: "Prompt 调试" },
  { id: "storage", label: "存储" },
  { id: "privacy", label: "隐私" },
  { id: "test", label: "测试" },
  { id: "about", label: "关于" },
];
```

- [ ] **Step 2: Import and render `PersonaSection`**

Add the import alongside other section imports:

```tsx
import { PersonaSection } from "@/pages/settings/sections/persona-section";
```

In the render block (inside the `<div className="min-w-0 flex-1">`), add right after the `ModelSection` line:

```tsx
          {active === "persona" ? <PersonaSection /> : null}
```

The full block should read:

```tsx
        <div className="min-w-0 flex-1">
          {active === "general" ? <GeneralSection /> : null}
          {active === "model" ? <ModelSection /> : null}
          {active === "persona" ? <PersonaSection /> : null}
          {active === "api-keys" ? <ApiKeysSection /> : null}
          {active === "prompt-debug" ? <PromptDebugSection /> : null}
          {active === "storage" ? <StorageSection /> : null}
          {active === "privacy" ? <PrivacySection /> : null}
          {active === "test" ? <TestSection /> : null}
          {active === "about" ? <AboutSection /> : null}
        </div>
```

- [ ] **Step 3: Run full frontend build + test**

Run: `pnpm build 2>&1 | tail -20`
Expected: `tsc -b` passes; `vite build` passes.

Run: `pnpm test 2>&1 | tail -20`
Expected: all tests pass (including the new persona-section tests).

- [ ] **Step 4: Manual smoke — open the app and flip a preset**

Run: `pnpm tauri dev`
Expected:
1. Settings → 人格 shows 4 preset cards + 自定义 radio
2. Current selection reflects `config.soul.preset` (default: 旁观的朋友 checked)
3. Clicking 同伙 saves immediately (toast shows no error, card highlights)
4. Clicking 自定义 reveals textarea prefilled with gentle body
5. Clearing textarea and clicking 保存自定义 shows error toast "自定义人格不能为空"
6. Typing text and saving persists; restart app, verify selection sticks

Note: suggestion text changes require proposition churn; verifying via actual push is out of scope for this smoke — the snapshot + unit tests in Task 3 cover prompt-level correctness.

- [ ] **Step 5: Commit**

```bash
git add src/pages/settings/settings-page.tsx
git commit -m "feat(settings): wire PersonaSection into settings nav"
```

---

## Self-Review Checks (performed while writing this plan)

- **Spec coverage** — all 11 spec sections trace to a task:
  - §2/§3 architecture + file layout → Task 1 (SOUL files) + Task 3 (resolve fn)
  - §4 config shape + normalize → Task 2
  - §5 runtime injection + override precedence → Task 3
  - §6 4 preset bodies → Task 1 (ship) + Task 5 (UI copy mirror)
  - §7 suggest.md edit → Task 1
  - §8 settings UI → Tasks 5 + 6
  - §9 testing plan → Task 2 (normalize tests), Task 3 (snapshots), Task 5 (UI tests)
  - §10 risks: 2000-char cap + override precedence both covered in Task 2 + Task 3
  - §11 implementation order matches task sequence 1→2→3→4→5→6
- **Placeholder scan** — no TBD / TODO / "similar to" references; every code block is complete.
- **Type consistency** — `SoulPreset` variants, `SoulConfig` field names, `resolve_soul_text` signature, and `SuggestionGenerator::new` parameter order are consistent across Tasks 2/3/4/5 and all referenced call sites.

---

## Execution

**Plan complete and saved to [`docs/superpowers/plans/2026-04-20-soul-md-personality.md`](2026-04-20-soul-md-personality.md). Two execution options:**

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
