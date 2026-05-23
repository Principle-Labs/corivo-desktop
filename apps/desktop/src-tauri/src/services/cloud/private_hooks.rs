#![cfg(feature = "corivo-cloud")]

compile_error!(
    "the corivo-cloud feature requires the closed-source private hooks; run corivo-app/scripts/prepare-submodule.mjs to materialize private-src/"
);
