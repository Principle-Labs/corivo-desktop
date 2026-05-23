//! Tauri command surface for the Stripe top-up flow.
//!
//! Frontend BillingDialog calls `billing_me` to render the current
//! balance + allowed top-up amounts, then `billing_start_checkout` to
//! kick off a Stripe Checkout Session. The Stripe URL is opened in the
//! user's default browser; payment confirmation comes back to the
//! corivo-api as a webhook (no desktop-side push), so the UI must
//! re-poll `billing_me` to surface the updated balance.
//!
//! Both commands delegate to `state.cloud.billing.*`. In the
//! open-source build the trait points at `NoopBillingService` and
//! everything returns `FeatureUnavailable`; the BillingDialog itself
//! is hidden via the frontend capability probe.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;

/// Snapshot returned by `billing_me` — what the UI needs to render the
/// "我的额度" dialog.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct BillingMe {
    /// USD remaining on the user's sub2api key (`quota - quota_used`).
    /// `null` when the account has no sub2api key bound yet (legacy
    /// rows pre-billing-MVP — they get a key on next login).
    pub balance: Option<f64>,
    /// Predefined top-up amounts the desktop should offer in the dialog.
    /// Mirrors `BILLING_ALLOWED_AMOUNTS_USD` from the corivo-api env.
    pub allowed_amounts_usd: Vec<f64>,
    /// Most recent payment row, if any. Used to show "上次充值" status.
    pub last_payment: Option<BillingLastPayment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct BillingLastPayment {
    /// "pending" | "paid" | "credited" | "failed". Surfaced verbatim;
    /// the UI maps it to a localized label.
    pub status: String,
    /// ISO-8601 timestamp set when the webhook successfully raised the
    /// sub2api quota. `null` for any non-credited row.
    pub credited_at: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
    /// Amount of USD this payment buys, encoded as a decimal string
    /// (Drizzle's MySQL decimal mode hands back strings).
    pub credit_usd: String,
}

/// Create a Stripe Checkout session for the given USD amount and open
/// the returned URL in the user's default browser. Returns once the
/// browser has been launched — payment confirmation arrives async via
/// the corivo-api webhook, so the caller must re-poll `billing_me`.
#[tauri::command]
pub async fn billing_start_checkout(
    state: State<'_, AppState>,
    app: AppHandle,
    amount_usd: f64,
) -> Result<(), TauriError> {
    let url = state.cloud.billing.start_checkout_url(amount_usd).await?;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| TauriError::Unknown {
            message: format!("Could not open browser: {e}"),
        })?;
    Ok(())
}

#[tauri::command]
pub async fn billing_me(state: State<'_, AppState>) -> Result<BillingMe, TauriError> {
    Ok(state.cloud.billing.me().await?)
}
