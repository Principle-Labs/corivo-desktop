//! Tagged error union returned from `#[tauri::command]` handlers.
//!
//! Tauri serializes the `Err(_)` side of a command's `Result` verbatim to the
//! frontend. Returning a typed enum with `#[serde(tag = "kind", ...)]` gives
//! the TypeScript side a discriminated union it can pattern-match on — rather
//! than the opaque string Tauri falls back to for errors that only implement
//! `Display`.
//!
//! Conversion rules:
//! - `CorivoError::Network`            → `Network` (regardless of provenance).
//! - `CorivoError::QuotaExceeded`      → `LlmRateLimited` (retry-after not yet
//!   plumbed through providers, so the field is always `None`).
//! - `CorivoError::AuthDenied`         → `AuthDenied` (preserves the
//!   rejected email so the login page can name it back to the user).
//! - `CorivoError::FeatureUnavailable` → `FeatureUnavailable` (the cloud
//!   capability the caller asked for isn't compiled into this build —
//!   open-source builds use this for any auth/billing/connectors call).
//! - `ExtractorError`                  → `Pipeline` (capture-pipeline
//!   extraction failures: I/O, AX/OCR errors).
//! - Anything else → `Unknown`.

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::{error::CorivoError, services::extractor::ExtractorError};

#[derive(Debug, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub enum TauriError {
    Network {
        message: String,
    },
    LlmRateLimited {
        message: String,
        #[ts(type = "number | null")]
        retry_after_ms: Option<u64>,
    },
    Pipeline {
        message: String,
        #[ts(type = "unknown | null")]
        detail: Option<Value>,
    },
    AuthDenied {
        message: String,
        email: Option<String>,
    },
    /// 当前构建没有这个 cloud capability。`feature` 是稳定标识符
    /// （"auth" / "billing" / ...），前端可以据此 fallback；`message`
    /// 是可直接展示的中文文案。开源版给所有 corivo-cloud 命令兜底用。
    FeatureUnavailable {
        feature: String,
        message: String,
    },
    Unknown {
        message: String,
    },
}

/// Peel the always-mapped `CorivoError` variants (`Network`,
/// `QuotaExceeded`, `AuthDenied`) and hand off anything else to
/// `fallback` so each caller can decide which provenance-tagged
/// variant to emit.
fn classify_corivo(
    err: CorivoError,
    fallback: impl FnOnce(CorivoError) -> TauriError,
) -> TauriError {
    match err {
        CorivoError::Network(message) => TauriError::Network { message },
        CorivoError::QuotaExceeded(message) => TauriError::LlmRateLimited {
            message,
            retry_after_ms: None,
        },
        CorivoError::AuthDenied { message, email } => TauriError::AuthDenied { message, email },
        CorivoError::FeatureUnavailable { feature, message } => TauriError::FeatureUnavailable {
            feature: feature.to_string(),
            message,
        },
        other => fallback(other),
    }
}

impl From<CorivoError> for TauriError {
    fn from(err: CorivoError) -> Self {
        classify_corivo(err, |e| TauriError::Unknown {
            message: e.to_string(),
        })
    }
}

impl From<ExtractorError> for TauriError {
    fn from(err: ExtractorError) -> Self {
        TauriError::Pipeline {
            message: err.to_string(),
            detail: None,
        }
    }
}
