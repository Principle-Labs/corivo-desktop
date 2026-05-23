//! 前端日志桥：让 webview 端的关键事件落到统一的 tracing 日志，
//! 而不是只停留在 webview 的 console（生产构建里看不到）。
//!
//! 用法（前端）：
//!     await invoke("log_frontend", {
//!       level: "error", target: "updater", message: "...", fields: { ... }
//!     });

use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, error, info, warn};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[tauri::command]
pub async fn log_frontend(
    level: LogLevel,
    target: String,
    message: String,
    fields: Option<Value>,
) -> Result<(), String> {
    let fields_str = match fields {
        Some(v) if !v.is_null() => format!(" fields={}", v),
        _ => String::new(),
    };
    match level {
        LogLevel::Debug => debug!(target = %target, "{}{}", message, fields_str),
        LogLevel::Info => info!(target = %target, "{}{}", message, fields_str),
        LogLevel::Warn => warn!(target = %target, "{}{}", message, fields_str),
        LogLevel::Error => error!(target = %target, "{}{}", message, fields_str),
    }
    Ok(())
}
