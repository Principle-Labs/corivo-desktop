//! Ordered registry of `FocusAdapter`s (spec §六).
//!
//! Order is meaningful: the registry consults `matches()` in insertion
//! order and returns the first hit. Per-app adapters go first, then
//! `OcrOnlyAdapter` (apps whose AX tree is known useless), and
//! `GenericAxAdapter` last as the catch-all so `find()` never returns
//! `None`.
//!
//! The generic adapter is also exposed via `generic()` so the
//! dispatcher can ask it to produce a body when a per-app adapter
//! returned `NeedsAxBody`.

use std::sync::Arc;

use super::{
    BrowserAdapter, FocusAdapter, GenericAxAdapter, ITerm2Adapter, JetBrainsAdapter, LarkAdapter,
    OcrOnlyAdapter, VsCodeAdapter,
};

pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn FocusAdapter>>,
    /// Direct handle to the generic AX adapter so the dispatcher can
    /// run it explicitly on `NeedsAxBody` outcomes without re-walking
    /// the registry. Populated alongside `adapters` in `builtin()`.
    generic: Arc<dyn FocusAdapter>,
}

impl AdapterRegistry {
    /// Per-app adapters first (order is meaningful — first match
    /// wins), then `OcrOnlyAdapter` for known-useless AX trees, then
    /// `GenericAxAdapter` as the catch-all. Adding a new app adapter
    /// is a single `Arc::new(...)` push above the OcrOnly entry; no
    /// dispatcher / schema changes required.
    pub fn builtin() -> Self {
        let generic: Arc<dyn FocusAdapter> = Arc::new(GenericAxAdapter::new());
        Self {
            adapters: vec![
                Arc::new(BrowserAdapter::new()),
                Arc::new(VsCodeAdapter::new()),
                Arc::new(JetBrainsAdapter::new()),
                Arc::new(LarkAdapter::new()),
                Arc::new(ITerm2Adapter::new()),
                Arc::new(OcrOnlyAdapter::new()),
                Arc::clone(&generic),
            ],
            generic,
        }
    }

    /// Test escape hatch — build a registry with an explicit roster
    /// plus an explicit generic adapter (the generic is always needed
    /// so callers can't forget it).
    #[cfg(test)]
    pub fn with(adapters: Vec<Arc<dyn FocusAdapter>>, generic: Arc<dyn FocusAdapter>) -> Self {
        Self { adapters, generic }
    }

    /// Pick the first adapter that handles `bundle_id`. Falls back to
    /// the generic adapter if nothing earlier matches.
    pub fn find(&self, bundle_id: &str) -> Arc<dyn FocusAdapter> {
        self.adapters
            .iter()
            .find(|a| a.matches(bundle_id))
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.generic))
    }

    /// Direct handle to the generic AX adapter. Dispatcher uses this
    /// to run an AX body extraction after a per-app adapter returned
    /// `NeedsAxBody`.
    pub fn generic(&self) -> Arc<dyn FocusAdapter> {
        Arc::clone(&self.generic)
    }

    pub fn len(&self) -> usize {
        self.adapters.len()
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::extractor::adapters::{AdapterContext, AdapterOutcome, FocusAdapter};
    use async_trait::async_trait;
    use serde_json::json;

    struct DummyAdapter {
        name: &'static str,
        matches_id: &'static str,
    }

    #[async_trait]
    impl FocusAdapter for DummyAdapter {
        fn name(&self) -> &'static str {
            self.name
        }
        fn matches(&self, bundle_id: &str) -> bool {
            bundle_id == self.matches_id
        }
        async fn extract(&self, _: &AdapterContext<'_>) -> super::super::Result<AdapterOutcome> {
            Ok(AdapterOutcome::Extracted {
                text: self.name.into(),
                payload: json!({}),
            })
        }
    }

    #[test]
    fn first_match_wins_over_later_entries() {
        let generic: Arc<dyn FocusAdapter> = Arc::new(GenericAxAdapter::new());
        let reg = AdapterRegistry::with(
            vec![
                Arc::new(DummyAdapter {
                    name: "chrome",
                    matches_id: "com.google.Chrome",
                }),
                Arc::clone(&generic),
            ],
            generic,
        );
        let hit = reg.find("com.google.Chrome");
        assert_eq!(hit.name(), "chrome");
    }

    #[test]
    fn missing_match_falls_through_to_generic() {
        let reg = AdapterRegistry::builtin();
        let hit = reg.find("com.unknown.App");
        assert_eq!(hit.name(), "generic_ax");
    }

    #[test]
    fn builtin_registry_terminates_on_generic_for_pathological_inputs() {
        let reg = AdapterRegistry::builtin();
        assert!(reg.len() >= 1);
        let hit = reg.find("");
        assert_eq!(hit.name(), "generic_ax");
    }

    #[test]
    fn ocr_only_adapter_wins_for_known_blacklist() {
        let reg = AdapterRegistry::builtin();
        assert_eq!(reg.find("com.figma.Desktop").name(), "ocr_only");
        assert_eq!(reg.find("dev.warp.Warp-Stable").name(), "ocr_only");
    }

    #[test]
    fn generic_handle_is_accessible() {
        let reg = AdapterRegistry::builtin();
        assert_eq!(reg.generic().name(), "generic_ax");
    }
}
