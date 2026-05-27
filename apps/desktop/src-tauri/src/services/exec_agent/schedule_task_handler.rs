//! JSON-RPC handlers for the `schedule_task` family of native
//! AgentTools (v1431).
//!
//! Four methods, all writing through `WorkflowStore`:
//!
//! | method                  | what the agent uses it for                       |
//! |-------------------------|--------------------------------------------------|
//! | `schedule_task`         | "remind me every day at 9 to review yesterday"   |
//! | `list_scheduled_tasks`  | "what reminders do I have set up?"               |
//! | `cancel_scheduled_task` | "cancel that morning review"                     |
//! | `update_scheduled_task` | "change the daily review to 8am"                 |
//!
//! Agent-created schedules land with `source = 'agent'` and
//! `created_by_thread_id = <current turn's thread>` so the
//! `/workflows` UI can render an "由 Corivo 自动创建" badge and the
//! user can audit which conversation prompted each one.
//!
//! The system prompt the agent stores becomes a future agent's system
//! prompt. The Trigger / tool_whitelist / max_turns shape mirrors
//! `WorkflowSaveSpec` so the user can later edit agent-created
//! schedules through the same UI.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::domain::workflow::{Trigger, WorkflowDefinition, WorkflowScheduleSource};
use crate::error::{CorivoError, Result};
use crate::services::scheduled_workflows::{UpsertSchedule, WorkflowStore};

/// Tools the agent may delegate to its scheduled task by default. Two
/// safe primitives: read (`memory_search`) + write to declarative
/// memory (`save_note`). Covers the "review / remind / journal" loop
/// that's 9 out of 10 user requests.
const DEFAULT_TOOL_WHITELIST: &[&str] = &["memory_search", "save_note"];
const DEFAULT_MAX_TURNS: u32 = 12;

/// `schedule_task` — create a new scheduled workflow.
///
/// Params (TypeBox-validated on the sidecar side; this handler is
/// defensive but trusts shapes):
/// ```text
/// {
///   name: string,
///   prompt: string,                       // system_prompt for the run
///   trigger: Trigger,                     // tagged JSON union
///   description?: string,
///   tools?: string[],                     // defaults DEFAULT_TOOL_WHITELIST
///   max_turns?: number,                   // defaults DEFAULT_MAX_TURNS
///   enabled?: boolean,                    // defaults true
/// }
/// ```
///
/// Returns `{ slug, next_run_at }`.
pub async fn schedule_task_handler(
    params: Value,
    store: Option<&Arc<WorkflowStore>>,
    thread_id: &str,
) -> Result<Value> {
    let store = require_store(store)?;

    let name = require_string(&params, "name")?;
    let prompt = require_string(&params, "prompt")?;
    let trigger = parse_trigger(&params)?;
    trigger
        .validate()
        .map_err(|e| CorivoError::Internal(format!("schedule_task: bad trigger: {e}")))?;

    let description = optional_string(&params, "description");
    let tools = optional_string_list(&params, "tools").unwrap_or_else(default_tool_whitelist);
    let max_turns = params
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|v| v.clamp(1, 50) as u32)
        .unwrap_or(DEFAULT_MAX_TURNS);
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let slug = store.generate_unique_slug(&name).await?;
    let definition = WorkflowDefinition {
        slug: slug.clone(),
        name,
        description,
        tool_whitelist: tools,
        max_turns,
        system_prompt: prompt,
    };
    store.write_definition(&definition)?;
    let schedule = store
        .upsert_schedule(UpsertSchedule {
            slug: slug.clone(),
            trigger,
            enabled,
            source: WorkflowScheduleSource::Agent,
            created_by_thread_id: Some(thread_id.to_string()),
        })
        .await?;

    Ok(json!({
        "slug": slug,
        "enabled": schedule.enabled,
        "next_run_at": schedule.next_run_at.map(|t| t.to_rfc3339()),
    }))
}

/// `list_scheduled_tasks` — return a slim summary the agent can show
/// the user. We deliberately don't dump the full system_prompt — the
/// agent typically just needs name + trigger + status to answer
/// "what's scheduled?".
pub async fn list_scheduled_tasks_handler(
    params: Value,
    store: Option<&Arc<WorkflowStore>>,
) -> Result<Value> {
    let store = require_store(store)?;
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("all");

    let definitions = store.scan_definitions();
    let schedules = store.list_schedules().await?;
    let mut out = Vec::new();
    for sched in schedules {
        match scope {
            "agent" if sched.source != WorkflowScheduleSource::Agent => continue,
            "user" if sched.source != WorkflowScheduleSource::User => continue,
            _ => {}
        }
        let def = definitions.iter().find(|d| d.slug == sched.slug);
        out.push(json!({
            "slug": sched.slug,
            "name": def.map(|d| d.name.clone()).unwrap_or(sched.slug.clone()),
            "description": def.and_then(|d| d.description.clone()),
            "trigger": sched.trigger,
            "enabled": sched.enabled,
            "source": sched.source.as_str(),
            "last_run_at": sched.last_run_at.map(|t| t.to_rfc3339()),
            "next_run_at": sched.next_run_at.map(|t| t.to_rfc3339()),
            "last_status": sched.last_status.map(|s| s.as_str()),
        }));
    }
    Ok(json!({ "tasks": out }))
}

/// `cancel_scheduled_task` — fully delete by slug. We match the IPC
/// surface (which also deletes the file) so the agent's cancel
/// matches what the user sees after "delete" in the UI.
pub async fn cancel_scheduled_task_handler(
    params: Value,
    store: Option<&Arc<WorkflowStore>>,
) -> Result<Value> {
    let store = require_store(store)?;
    let slug = require_string(&params, "slug")?;

    let existed = store.get_schedule(&slug).await?.is_some()
        || store.load_definition(&slug).is_some();
    if !existed {
        return Ok(json!({ "cancelled": false, "reason": "slug not found" }));
    }
    store.delete_schedule(slug.clone()).await?;
    store.delete_definition(&slug)?;
    Ok(json!({ "cancelled": true, "slug": slug }))
}

/// `update_scheduled_task` — patch any subset of fields on an existing
/// slug. Source is preserved (the store's upsert path explicitly
/// never overwrites it).
pub async fn update_scheduled_task_handler(
    params: Value,
    store: Option<&Arc<WorkflowStore>>,
) -> Result<Value> {
    let store = require_store(store)?;
    let slug = require_string(&params, "slug")?;

    let mut definition = store
        .load_definition(&slug)
        .ok_or_else(|| CorivoError::Internal(format!("update_scheduled_task: unknown slug {slug}")))?;
    let existing_schedule = store
        .get_schedule(&slug)
        .await?
        .ok_or_else(|| CorivoError::Internal(format!("update_scheduled_task: no schedule for {slug}")))?;

    if let Some(name) = optional_string(&params, "name") {
        definition.name = name;
    }
    if let Some(desc) = optional_string(&params, "description") {
        definition.description = Some(desc);
    }
    if let Some(prompt) = optional_string(&params, "prompt") {
        definition.system_prompt = prompt;
    }
    if let Some(tools) = optional_string_list(&params, "tools") {
        definition.tool_whitelist = tools;
    }
    if let Some(max_turns) = params
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|v| v.clamp(1, 50) as u32)
    {
        definition.max_turns = max_turns;
    }

    let trigger = if params.get("trigger").is_some() {
        let t = parse_trigger(&params)?;
        t.validate()
            .map_err(|e| CorivoError::Internal(format!("update_scheduled_task: {e}")))?;
        t
    } else {
        existing_schedule.trigger.clone()
    };
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(existing_schedule.enabled);

    store.write_definition(&definition)?;
    let schedule = store
        .upsert_schedule(UpsertSchedule {
            slug: slug.clone(),
            trigger,
            enabled,
            source: existing_schedule.source,
            created_by_thread_id: existing_schedule.created_by_thread_id,
        })
        .await?;

    Ok(json!({
        "slug": slug,
        "enabled": schedule.enabled,
        "next_run_at": schedule.next_run_at.map(|t| t.to_rfc3339()),
    }))
}

// ---------------------------------------------------------------------------
// Param helpers
// ---------------------------------------------------------------------------

fn require_store<'a>(
    store: Option<&'a Arc<WorkflowStore>>,
) -> Result<&'a Arc<WorkflowStore>> {
    store.ok_or_else(|| {
        CorivoError::Internal("schedule_task: workflow store not initialized".into())
    })
}

fn require_string(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CorivoError::Internal(format!("schedule_task: missing '{key}'")))
}

fn optional_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn optional_string_list(params: &Value, key: &str) -> Option<Vec<String>> {
    params.get(key).and_then(Value::as_array).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

fn parse_trigger(params: &Value) -> Result<Trigger> {
    let raw = params
        .get("trigger")
        .ok_or_else(|| CorivoError::Internal("schedule_task: missing 'trigger'".into()))?;
    serde_json::from_value::<Trigger>(raw.clone())
        .map_err(|e| CorivoError::Internal(format!("schedule_task: bad trigger: {e}")))
}

fn default_tool_whitelist() -> Vec<String> {
    DEFAULT_TOOL_WHITELIST.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Reminder: keep DEFAULT_TOOL_WHITELIST and DEFAULT_MAX_TURNS in sync
// with the TypeScript tool description (packages/agent/src/native-tools/
// schedule-task.ts) so the agent's intuition matches what it actually
// gets at run-time.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::apply_migrations;
    use crate::db::pool::test_in_memory_pool;
    use tempfile::TempDir;

    fn store_with_schema() -> (TempDir, Arc<WorkflowStore>) {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        apply_migrations(&conn).unwrap();
        drop(conn);
        let dir = TempDir::new().unwrap();
        let store = WorkflowStore::new(pool, dir.path());
        store.ensure_root().unwrap();
        (dir, store)
    }

    /// Seed a minimal `chat_threads` row so the schedule's
    /// `created_by_thread_id` FK is satisfied. The schedule_task
    /// handler is always called from inside a real turn in production,
    /// so the FK invariant is normally trivially satisfied.
    async fn seed_thread(store: &Arc<WorkflowStore>, id: &str) {
        use crate::db::pool::run_blocking;
        let id_owned = id.to_string();
        run_blocking(store.pool_for_tests(), move |conn| {
            conn.execute(
                "INSERT INTO chat_threads
                     (id, bound_model_id, bound_api_shape)
                 VALUES (?1, 'test-model', 'anthropic')",
                rusqlite::params![id_owned],
            )
            .map_err(|e| {
                crate::error::CorivoError::Internal(format!("seed_thread: {e}"))
            })?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn schedule_task_writes_definition_and_schedule_with_agent_source() {
        let (_dir, store) = store_with_schema();
        seed_thread(&store, "T-thread-1").await;
        let params = json!({
            "name": "晚上回顾",
            "prompt": "回顾今天发生的事情",
            "trigger": { "kind": "daily", "hour": 22, "minute": 0, "tz": "Asia/Shanghai" },
        });
        let result = schedule_task_handler(params, Some(&store), "T-thread-1")
            .await
            .unwrap();
        let slug = result["slug"].as_str().unwrap().to_string();
        assert!(slug.starts_with("task") || !slug.is_empty()); // kebab fallback safe

        let schedule = store.get_schedule(&slug).await.unwrap().unwrap();
        assert_eq!(schedule.source, WorkflowScheduleSource::Agent);
        assert_eq!(
            schedule.created_by_thread_id.as_deref(),
            Some("T-thread-1")
        );
        let def = store.load_definition(&slug).unwrap();
        assert_eq!(def.name, "晚上回顾");
        assert_eq!(
            def.tool_whitelist,
            vec!["memory_search".to_string(), "save_note".to_string()]
        );
    }

    #[tokio::test]
    async fn schedule_task_collisions_get_unique_suffix() {
        let (_dir, store) = store_with_schema();
        seed_thread(&store, "T1").await;
        let params = json!({
            "name": "Review",
            "prompt": "...",
            "trigger": { "kind": "interval", "minutes": 60 },
        });
        let r1 = schedule_task_handler(params.clone(), Some(&store), "T1")
            .await
            .unwrap();
        let r2 = schedule_task_handler(params.clone(), Some(&store), "T1")
            .await
            .unwrap();
        assert_ne!(r1["slug"], r2["slug"]);
    }

    #[tokio::test]
    async fn schedule_task_rejects_missing_name() {
        let (_dir, store) = store_with_schema();
        let params = json!({
            "prompt": "...",
            "trigger": { "kind": "interval", "minutes": 60 },
        });
        let err = schedule_task_handler(params, Some(&store), "T1").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn cancel_scheduled_task_removes_both_layers() {
        let (_dir, store) = store_with_schema();
        seed_thread(&store, "T1").await;
        let params = json!({
            "name": "Done",
            "prompt": "...",
            "trigger": { "kind": "interval", "minutes": 60 },
        });
        let created = schedule_task_handler(params, Some(&store), "T1")
            .await
            .unwrap();
        let slug = created["slug"].as_str().unwrap().to_string();

        let result = cancel_scheduled_task_handler(
            json!({ "slug": slug.clone() }),
            Some(&store),
        )
        .await
        .unwrap();
        assert_eq!(result["cancelled"], true);
        assert!(store.get_schedule(&slug).await.unwrap().is_none());
        assert!(store.load_definition(&slug).is_none());
    }

    #[tokio::test]
    async fn update_scheduled_task_preserves_source() {
        let (_dir, store) = store_with_schema();
        seed_thread(&store, "T1").await;
        let create = schedule_task_handler(
            json!({
                "name": "Daily",
                "prompt": "...",
                "trigger": { "kind": "daily", "hour": 9, "minute": 0, "tz": "Asia/Shanghai" },
            }),
            Some(&store),
            "T1",
        )
        .await
        .unwrap();
        let slug = create["slug"].as_str().unwrap().to_string();

        update_scheduled_task_handler(
            json!({
                "slug": slug.clone(),
                "trigger": { "kind": "daily", "hour": 8, "minute": 0, "tz": "Asia/Shanghai" },
            }),
            Some(&store),
        )
        .await
        .unwrap();

        let after = store.get_schedule(&slug).await.unwrap().unwrap();
        // Source must still be Agent — the user-edit path doesn't
        // demote it, and the agent-edit path obviously shouldn't either.
        assert_eq!(after.source, WorkflowScheduleSource::Agent);
        match after.trigger {
            Trigger::Daily { hour, .. } => assert_eq!(hour, 8),
            other => panic!("expected Daily trigger, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_scheduled_tasks_filters_by_source() {
        let (_dir, store) = store_with_schema();
        seed_thread(&store, "T1").await;
        // User-created via direct store API.
        store.write_definition(&WorkflowDefinition {
            slug: "user-one".into(),
            name: "User-managed".into(),
            description: None,
            tool_whitelist: vec![],
            max_turns: 5,
            system_prompt: "x".into(),
        }).unwrap();
        store
            .upsert_schedule(UpsertSchedule {
                slug: "user-one".into(),
                trigger: Trigger::Interval { minutes: 60 },
                enabled: true,
                source: WorkflowScheduleSource::User,
                created_by_thread_id: None,
            })
            .await
            .unwrap();
        // Agent-created via handler.
        schedule_task_handler(
            json!({
                "name": "Agent-managed",
                "prompt": "...",
                "trigger": { "kind": "interval", "minutes": 60 },
            }),
            Some(&store),
            "T1",
        )
        .await
        .unwrap();

        let all = list_scheduled_tasks_handler(json!({}), Some(&store))
            .await
            .unwrap();
        assert_eq!(all["tasks"].as_array().unwrap().len(), 2);

        let agent_only = list_scheduled_tasks_handler(
            json!({ "scope": "agent" }),
            Some(&store),
        )
        .await
        .unwrap();
        assert_eq!(agent_only["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(agent_only["tasks"][0]["source"], "agent");
    }
}
