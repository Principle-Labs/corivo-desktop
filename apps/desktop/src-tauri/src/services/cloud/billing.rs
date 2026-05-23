//! Stripe top-up capability.
//!
//! Closed impl talks to the private Corivo billing API. The
//! open-source noop hides the entire dialog — there's no payment
//! surface in the OSS build by design.

use async_trait::async_trait;

use crate::error::Result;

// Task #4 will move BillingMe / BillingLastPayment under this module.
// They're IPC types today (#[ts(export)] from commands::billing); since
// the OSS shared-types bundle won't include them, the migration is
// also when they leave packages/shared-types/src/generated/.
pub use crate::commands::billing::{BillingLastPayment, BillingMe};

#[async_trait]
pub trait BillingService: Send + Sync {
    /// Whether this build has a real billing backend. Open-source =
    /// false, hides the entire BillingDialog plus the余额 chip.
    fn is_available(&self) -> bool {
        false
    }

    /// Snapshot used by the "我的额度" dialog: current balance, allowed
    /// top-up amounts, last payment status.
    async fn me(&self) -> Result<BillingMe>;

    /// Create a Stripe Checkout session for the given USD amount.
    /// Returns the URL the desktop should open in the user's default
    /// browser. The webhook handles credit reconciliation; the UI
    /// re-polls `me()` to surface the updated balance.
    async fn start_checkout_url(&self, amount_usd: f64) -> Result<String>;
}
