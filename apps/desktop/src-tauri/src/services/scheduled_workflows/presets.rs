//! First-boot preset workflows.
//!
//! On a fresh `$APPDATA/corivo/workflows/` directory we seed a single
//! starter workflow (`daily-review`) so the `/workflows` page has
//! something useful out of the box. The preset is `enabled = false`
//! by default — the user opts in from the list UI.
//!
//! Re-seeding policy: a `.seeded` marker file inside the workflows
//! root suppresses subsequent runs. If the user deletes
//! `daily-review/` on purpose, the marker stays and we don't re-create.
//! Wiping the marker (or the whole `corivo/` dir) restores first-boot
//! behaviour on next launch.

use std::fs;

use crate::domain::workflow::{WorkflowDefinition, WorkflowNotifyPolicy};
use crate::services::scheduled_workflows::WorkflowStore;

const SEED_MARKER: &str = ".seeded";

/// Idempotent seed pass. Logs but doesn't fail the boot when a write
/// errors — a missing preset is recoverable; a stalled boot is not.
pub fn seed_defaults(store: &WorkflowStore) {
    let marker = store.root().join(SEED_MARKER);
    if marker.exists() {
        return;
    }
    for def in default_workflows() {
        if store.load_definition(&def.slug).is_some() {
            tracing::debug!(slug = %def.slug, "scheduled_workflow.seed_skip_existing");
            continue;
        }
        match store.write_definition(&def) {
            Ok(()) => tracing::info!(slug = %def.slug, "scheduled_workflow.seed_wrote"),
            Err(error) => {
                tracing::warn!(
                    slug = %def.slug,
                    ?error,
                    "scheduled_workflow.seed_write_failed"
                );
            }
        }
    }
    if let Err(error) = fs::write(&marker, b"v1\n") {
        tracing::warn!(?error, "scheduled_workflow.seed_marker_write_failed");
    }
}

fn default_workflows() -> Vec<WorkflowDefinition> {
    vec![daily_review()]
}

/// `daily-review` — a starter workflow that pulls yesterday's frames
/// and writes a ~200 字 Markdown summary into the user's notes. Comes
/// shipped `enabled = false`; the user attaches a trigger and flips
/// the switch from the `/workflows` page.
fn daily_review() -> WorkflowDefinition {
    WorkflowDefinition {
        slug: "daily-review".to_string(),
        name: "每日回顾".to_string(),
        description: Some(
            "拉取昨日的 frames 与对话，生成约 200 字的 Markdown 工作纪要并存入笔记。".to_string(),
        ),
        tool_whitelist: vec!["memory_search".to_string(), "save_note".to_string()],
        max_turns: 12,
        system_prompt: DAILY_REVIEW_PROMPT.to_string(),
        // The whole point of a daily review is to surface it — keep
        // the default loud policy.
        notify_policy: WorkflowNotifyPolicy::Always,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;
    use crate::db::Database;
    use tempfile::TempDir;

    fn store_in_tempdir() -> (TempDir, std::sync::Arc<WorkflowStore>) {
        // The presets tests don't actually hit SQLite, but
        // WorkflowStore::new wants a DbPool — use a fresh in-memory
        // one so we don't pollute prod schema.
        let pool = test_in_memory_pool().unwrap();
        let dir = TempDir::new().unwrap();
        let store = WorkflowStore::new(pool, dir.path());
        store.ensure_root().unwrap();
        (dir, store)
    }

    // Touch Database so the test_in_memory_pool import resolves
    // identically to production code paths even without a real DB.
    #[allow(dead_code)]
    fn _link_db_into_tests(_: &Database) {}

    #[test]
    fn seeds_daily_review_on_first_run() {
        let (_dir, store) = store_in_tempdir();
        seed_defaults(&store);
        let loaded = store.load_definition("daily-review").unwrap();
        assert_eq!(loaded.name, "每日回顾");
        assert!(loaded.tool_whitelist.contains(&"save_note".to_string()));
        assert!(store.root().join(".seeded").exists());
    }

    #[test]
    fn seed_is_idempotent() {
        let (_dir, store) = store_in_tempdir();
        seed_defaults(&store);
        // Edit the file so we can tell whether the second seed
        // overwrote it (it must not).
        let manifest = store.root().join("daily-review/WORKFLOW.md");
        let edited = "---\nname: 我自己改过的\n---\n自定义提示\n";
        std::fs::write(&manifest, edited).unwrap();
        seed_defaults(&store);
        let still_edited = std::fs::read_to_string(&manifest).unwrap();
        assert!(still_edited.contains("我自己改过的"));
    }

    #[test]
    fn marker_blocks_resurrection_after_user_delete() {
        let (_dir, store) = store_in_tempdir();
        seed_defaults(&store);
        // Simulate the user deleting daily-review on purpose.
        store.delete_definition("daily-review").unwrap();
        seed_defaults(&store);
        assert!(
            store.load_definition("daily-review").is_none(),
            "marker should suppress re-seeding after user delete"
        );
    }
}

const DAILY_REVIEW_PROMPT: &str = r#"你是 Corivo 的每日回顾助手。每次运行时,请按下面的步骤工作:

1. 用 `memory_search` 查询昨天（{{date_yesterday}}）的活动。
   - 关键词建议从空开始,让全量召回。
   - 必要时缩短时间窗口、用更具体的关键词重试一次。

2. 阅读召回结果,挑出昨天最值得记下的 3–5 件事。判断标准:
   - 真正动手做了的事(代码、决策、对话、文档),不要罗列被动信息流。
   - 重复出现 / 持续了较长时间的话题优先。
   - 单帧偶然画面不算。

3. 写一份 Markdown 纪要,结构如下:
   ```markdown
   # 每日回顾 · {{date_yesterday}}

   ## 完成
   - …

   ## 进行中
   - …

   ## 想到的下一步
   - …
   ```
   总长度控制在 200 字左右。空小节直接省略,不要写"无"或占位文本。

4. 用 `save_note(content="<上面整段 markdown>", scope="global", source="agent_inferred")`
   保存这条纪要。一次运行写一条 note,不要拆成多条。

5. 把同样的 markdown 内容作为最终回复返回,方便用户在运行历史里直接看到。

注意:
- 没有可写内容时返回一句"昨日无显著活动"并跳过 save_note,不要硬凑。
- 不调用未在白名单内的工具。"#;
