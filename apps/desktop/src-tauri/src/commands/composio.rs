#![cfg(feature = "corivo-cloud")]

compile_error!(
    "the corivo-cloud feature requires the closed-source Composio command implementation; run corivo-app/scripts/prepare-submodule.mjs to materialize private-src/commands/composio.rs"
);
