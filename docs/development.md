# Development and Validation

## Before editing

1. Read [`AGENTS.md`](../AGENTS.md).
2. Read the affected directory's `module_boundary.md`.
3. Check [Architecture](architecture.md) and
   [Semantic project layout](semantic-project-layout.md) when ownership or
   dependencies may change.
4. Identify compatibility requirements for CLI, persisted data, and wire contracts.

Use stable Rust, pnpm, and a supported Node.js release. Install frontend
dependencies with:

```bash
pnpm --dir interfaces/web install --frozen-lockfile
```

## Focused validation

Rust formatting and compilation:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
```

Architecture and contract checks:

```bash
python3 scripts/architecture_guard.py --self-test
scripts/module_boundary_check.sh
scripts/test_contract_check.sh
```

Rust tests:

```bash
cargo test --workspace --locked -- --test-threads=1
```

Web tests and tracked production assets:

```bash
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build
git diff --exit-code -- interfaces/web/dist
```

Performance-sensitive changes:

```bash
scripts/performance_guard.sh
```

Use the narrowest relevant checks while iterating, then run the complete gate
before delivery.

## Complete production gate

```bash
scripts/ci.sh
```

This is the authoritative local CI sequence. Do not report a change as fully
validated when an applicable step was skipped or failed; state the missing gate
and remaining risk explicitly.

## Change rules

- Fix the owning layer rather than copying semantics into a caller.
- Keep queues, retries, logs, event windows, and stores bounded.
- Preserve structured errors and fail closed for ownership, permission,
  destructive action, process identity, and unsupported targets.
- Update code, tests, contracts, architecture guards, and docs together when
  behavior changes.
- Web source changes must include the rebuilt `interfaces/web/dist` output and an
  explanation of generated differences.
- Do not commit secrets, private paths, user content, dependency directories,
  temporary diagnostics, or debug artifacts.
- Do not weaken or delete failing tests to manufacture a green result.

## Commits and release work

Keep commits scoped and inspect `git diff --check` plus `git status` before
committing. Pushing, tagging, publishing, and rewriting remote history require
explicit authorization. Release procedure and smoke checks are documented in
[Release management](release-management.md) and
[Manual release smoke checklist](manual-release-smoke.md).
