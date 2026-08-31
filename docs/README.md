# TimemAi Documentation

This directory separates product operation, architecture contracts, development
validation, protocols, and historical release material. The top-level
[README](../README.md) is the product entry point.

## Start here

| Goal | Document |
|---|---|
| Install, configure, update, or uninstall | [Install and configuration](install-and-configuration.md) |
| Understand the current system | [Architecture](architecture.md) |
| Understand directory and dependency ownership | [Semantic project layout](semantic-project-layout.md) |
| Build, test, and submit changes | [Development and validation](development.md) |
| Test the product manually | [测试人员手册（中文）](tester-handbook.zh-CN.md) |

## Architecture and protocols

- [Architecture](architecture.md) — current 2.0 system overview and invariants.
- [Semantic project layout](semantic-project-layout.md) — physical ownership and
  compile-time dependency direction.
- [Core/UI topic protocol](core-ui-topic-protocol.md) — typed Core-to-Interface topics.
- [Turn state projection architecture](turn-state-projection-architecture.md) —
  authoritative Turn projections.
- [Capability system](capability-system.md) — capability manifests, tools, and execution.
- [Run-bash job supervision](run-bash-job-supervision.md) — process and job lifecycle.
- [Chat search and favorites design](chat-search-favorites-design.md) — storage and UI design.

## Web contracts and diagnostics

- [Web reliability test matrix](web_reliability_test_matrix.md)
- [Web UI feature test matrix](web-ui-feature-test-matrix.md)
- [Web performance tracing](web-performance-tracing.md)
- [Windows support matrix](windows-support-matrix.md)

## Quality and delivery

- [Development and validation](development.md)
- [Test strategy](test-strategy.md)
- [Feature and test management](feature-test-management.md)
- [Release management](release-management.md)
- [Manual release smoke checklist](manual-release-smoke.md)

## Historical material

Release notes and audits describe the state of a particular release; they are
not the current architecture contract:

- [Release notes v2.0.0](release-notes-v2.0.0.md)
- [Release notes v1.3.0](release-notes-v1.3.0.md)
- [Release notes v1.2.0](release-notes-v1.2.0.md)
- [Release notes v1.1.3](release-notes-v1.1.3.md)
- [Earlier release notes](release-notes-v1.0.0.md)
- [v0.5 release audit](v0.5-release-audit.md)
- [KVC optimization report](kvc-optimization-report.md)
