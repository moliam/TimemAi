# Core Platform Boundary

`core/platform` is the only Core module that owns operating-system policy and
low-level process primitives shared across Timem hosts.

## Layout

- `src/api.rs`: stable, UI-neutral platform API consumed by Core.
- `src/shared.rs`: Unix primitives shared by macOS and Linux.
- `src/macos.rs`: macOS policy and kernel-derived process identity.
- `src/linux.rs`: Linux policy and `/proc`-derived process identity.

## Rules

- This crate must not depend on an Interface or Bridge.
- `agent_core` may consume only the public API; it must not duplicate platform
  selection or Unix process-group primitives.
- Target-specific modules compile only on their matching target.
- Unsupported targets fail closed for ownership/destructive decisions.
- Platform behavior changes require tests under `core/platform/tests`.
- Windows is intentionally outside the first-stage layout and must not be added
  without an explicit design and test matrix.
