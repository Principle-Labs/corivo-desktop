//! 业务埋点桥：把前端关心的"用户做了 X 动作"事件交给 telemetry trait
//! 处理，作为独立 event 出现在 dashboard。区别于 [`log::log_frontend`]：
//! 那个走 tracing layer，info 级只会沉淀为 breadcrumb（必须有一个
//! error 才会被打包带出去）；这里 telemetry trait 在 corivo 构建里
//! 直接 `capture_message` 一条独立 message event，永远可见。
//!
//! 在开源构建里 trait 指向 [`NoopTelemetryService`]，整个命令安静地
//! `Ok(())`——埋点不应该影响用户操作，更不应该返回错误。
//!
//! 用法（前端）：
//!     await invoke("track_event", { args: { name: "billing_dialog.opened" } });
//!     await invoke("track_event", {
//!       args: { name: "billing_checkout.started", properties: { amount_usd: 10 } }
//!     });

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;

#[derive(Debug, Deserialize)]
pub struct TrackEventArgs {
    pub name: String,
    #[serde(default)]
    pub properties: Option<Value>,
}

#[tauri::command]
pub async fn track_event(
    state: State<'_, AppState>,
    args: TrackEventArgs,
) -> Result<(), TauriError> {
    // 把 `properties` 的 JSON object 摊平成 BTreeMap<String, Value>。
    // 非 object 的 properties（数组、纯字符串）我们直接丢弃——前端没
    // 这种用法，留个 sanity check 而已。
    let properties = match args.properties {
        Some(Value::Object(map)) => Some(map.into_iter().collect::<BTreeMap<_, _>>()),
        _ => None,
    };
    // Fire-and-forget on the caller side; trait impl handles the
    // success channel itself (noop returns Ok unconditionally, corivo
    // impl wraps sentry and never errs).
    state
        .cloud
        .telemetry
        .track_event(args.name, properties)
        .await?;
    Ok(())
}
