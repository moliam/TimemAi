# Windows Support Matrix

Windows support is introduced by layer. A lower layer compiling does not imply that an Interface,
host, installer, or end-to-end product flow is supported.

| Layer | Status | Executable evidence | Remaining gate |
| --- | --- | --- | --- |
| `core/platform` | Implemented, awaiting native revalidation | macOS `cargo test -p timem_platform`; `cargo check -p timem_platform --target x86_64-pc-windows-msvc` | Native Windows tests exercise process, filesystem lease, command, and launch policy behavior. |
| `core/agent` storage and time primitives | Adapted, awaiting native revalidation | macOS `cargo test -p agent_core`; Platform-backed lease, private-file, home, and local-time APIs | Agent storage tests compile and pass natively on Windows. |
| `core/agent` local execution and jobs | Adapted, awaiting native revalidation | macOS `cargo test -p agent_core`; platform-neutral command, capability, tool self-test, and background-job tests | Agent tests compile and pass natively on Windows. macOS cross-check currently stops in bundled SQLite C compilation before full Agent validation. |
| `bridges/in_process` | Platform-neutral contract exists | Existing Bridge tests on supported development hosts | Revalidated as part of a Windows Shell test run. |
| `interfaces/shell` | Adapted, awaiting native revalidation | macOS `cargo test -p timem_shell`; target-specific Unix/Windows terminal adapters and platform-neutral Shell input tests | Shell tests compile and pass natively on Windows. The macOS Windows-target cross-check currently stops in bundled SQLite and Oniguruma C compilation before full Shell validation. |
| `timem_web` / HTTP host | Adapted, awaiting native revalidation | macOS `cargo test -p timem_web`; Windows launcher lifecycle adapter; Platform-backed random and instance lease | Web tests and HTTP lifecycle smoke pass natively on Windows. The current macOS Windows-target cross-check stops in bundled SQLite C compilation because the MSVC target standard-library headers are unavailable, before full Web Rust validation. |
| Install, uninstall, release packaging | Adapted, awaiting native revalidation | `install.ps1`, `uninstall.ps1`, Unix-side delivery contract checks, and a PowerShell contract test ready for native execution | PowerShell install tests, release build, and packaged install/uninstall smoke pass natively on Windows. |

## Platform contract

The Windows Platform backend owns:

- Windows configuration, home, browser, terminal, and version policy;
- secure randomness and file open/share policy;
- script interpreter selection and sanitized child environments;
- process liveness, creation-time identity, parent/child checks, process-tree termination, and new
  process-group creation.

Unsupported or unverified upper layers must not infer product support from the presence of
`core/platform/src/windows`. Each row changes status only with its own executable evidence.
