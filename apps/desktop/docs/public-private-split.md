# Public / Private Split Contract

Corivo desktop is developed from two repositories:

- `corivo-desktop` is the public repository. It owns the shared desktop
  application, local-first behavior, stable IPC / trait surfaces, and noop
  implementations.
- `corivo-app` is the private overlay. It pins `corivo-desktop` as a submodule
  and materializes closed-source implementations into cfg-gated module paths
  before closed-build commands run.

The public repository must always build and test without the private overlay.
The private repository may add, remove, or replace features on top of a pinned
public revision, but it should not require a private branch inside the public
repository.

## Invariants

- The default public build keeps `feature = "corivo-cloud"` off.
- Cloud behavior is accessed through stable trait objects under
  `src-tauri/src/services/cloud/`.
- Every public capability has a noop implementation that either returns a
  local-only value or `CorivoError::FeatureUnavailable`.
- Public UI checks `get_capabilities` before rendering cloud-only entry points.
- Public source must not contain closed Corivo API routes, token-rotation
  implementation, hosted Composio implementation, updater registration, signing
  endpoints, Sentry DSNs, or secrets-shaped constants.
- If a closed build needs a concrete implementation at a public module path, the
  public file is only a cfg-gated compile-time stub. The private overlay replaces
  it during materialization.

## Public Update Flow

Public changes should land in `corivo-desktop` first. The private repository
then bumps its submodule pointer and reruns the materialization step:

```bash
pnpm app:prepare
pnpm app:check
```

This keeps the public repository able to move independently. If a public update
changes a trait, the closed overlay breaks at compile time until the private
implementation is updated against the new surface.

## Adding Or Removing Private Features

To add a private feature cleanly:

1. Add the stable public trait / DTO / command surface.
2. Add noop behavior and capability gating in public code.
3. Keep the public build green with the feature disabled.
4. Add the concrete implementation under `corivo-app/private-src/`.
5. Materialize the private file from `prepare-submodule.mjs`.
6. Verify both the public default build and the closed feature build.

To remove a private feature, delete or stop materializing only the private
implementation first. The public trait/noop surface may remain until the UI and
IPC surface can be retired safely.
