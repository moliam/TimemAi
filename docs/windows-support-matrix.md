# Windows Support Matrix

Windows support is introduced by layer. A lower layer compiling does not imply that an Interface,
host, installer, or end-to-end product flow is supported.

| Layer | Status | Executable evidence | Remaining gate |
| --- | --- | --- | --- |
| `core/platform` | Implemented, awaiting native revalidation | macOS `cargo test -p timem_platform`; `cargo check -p timem_platform --target x86_64-pc-windows-msvc` | Native Windows tests exercise process, filesystem lease, command, and launch policy behavior. |
| `core/agent` storage and time primitives | Adapted, awaiting native revalidation | macOS `cargo test -p agent_core`; Platform-backed lease, private-file, home, and local-time APIs | Agent storage tests compile and pass natively on Windows. |
| `core/agent` local execution and jobs | Not yet adapted | None in this slice | Agent command, capability, tool self-test, and background-job tests compile and pass natively on Windows. |
| `bridges/in_process` | Platform-neutral contract exists | Existing Bridge tests on supported development hosts | Revalidated as part of a Windows Shell test run. |
| `interfaces/shell` | Not yet supported on Windows | None in this slice | Windows console/input adapter and Shell tests pass natively. |
| `timem_web` / HTTP host | Not yet supported on Windows | None in this slice | Windows lifecycle adapter, Web tests, and HTTP smoke pass natively. |
| Install, uninstall, release packaging | Not yet supported on Windows | None in this slice | PowerShell install tests and a packaged install/uninstall smoke pass natively. |

## Platform contract

The Windows Platform backend owns:

- Windows configuration, home, browser, terminal, and version policy;
- secure randomness and file open/share policy;
- script interpreter selection and sanitized child environments;
- process liveness, creation-time identity, parent/child checks, process-tree termination, and new
  process-group creation.

Unsupported or unverified upper layers must not infer product support from the presence of
`core/platform/src/windows`. Each row changes status only with its own executable evidence.
