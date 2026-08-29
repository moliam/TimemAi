# Install and Configuration

This page keeps operational details out of the top-level README while preserving
the full setup reference.

## Install

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

Timem supports macOS and Linux. Windows is not supported yet.

`install.sh` checks platform prerequisites:

- macOS: Xcode Command Line Tools and `curl`.
- Linux: `cc`, `make`, `curl`, `pkg-config`, and `ca-certificates`; when
  possible it installs missing packages through the system package manager.

If Rust/cargo is missing, the installer installs the Rust toolchain with
rustup. Cargo 1.78+ is required. To disable automatic Rust install/update:

```bash
TIMEM_SHELL_SKIP_RUST_INSTALL=1 ./install.sh
```

The installer runs:

```bash
cargo fetch --locked
cargo build --locked -p timem_shell -p timem_web --release
```

It installs:

- `timem-web`: recommended local browser UI with embedded production assets
- `timem`: optional terminal UI command
- `timem-native-rs`: terminal release binary used by the `timem` wrapper
- `resources/reminder_tips.json`: runtime-loaded default reminder schedules, normally under `~/.local/share/timem/resources`

The completion message leads with `timem-web`. No env file is required to open
the Web UI; model and API credentials can be configured in the browser. Env
files remain available for terminal use, automation, or defaults for new Web
Sessions.

`TIMEM_SHELL_INSTALL_DIR` changes the binary directory. Resources follow the same prefix at `../share/timem/resources` unless `TIMEM_RESOURCES_DIR` is set explicitly. User-level `reminder_tips.json` overrides are separate and are never overwritten by installation.

Binary updates are installed with an atomic file replacement. This allows
`./install.sh` to update an installation even while an older `timem-web`
process is still running, without invalidating the executable inode used by
that process on macOS. Restart the old process to use the newly installed
version.

Release users do not need Node.js or a separate assistant-ui checkout. Node/pnpm
are only needed for frontend development.

## Recommended Start: Timem Web

Start the installed Web host with one command:

```bash
timem-web
```

The loopback-only local UI opens without an access token or model credentials at
process startup. Only `--public` access requires the token printed in its URL.
Click the current model name in the upper-left header and configure
the API key, model, API protocol, Base URL, and token limits for the selected
Session. Configuration is Session-owned: changing one Session does not change
another Session's endpoint or model.

Use environment variables below when supplying defaults for new Sessions,
running the terminal UI, or automating startup. They are optional for opening
and configuring Timem Web.

## Env Files

Timem reads process environment variables. It does not load env files
implicitly.

```bash
cp env_template env
$EDITOR env
source /path/to/your/env
```

For Shell startup and Session resume, configuration precedence is:

```text
command-line option > non-empty process environment > restored Session cache > default
```

This means `source env` intentionally refreshes a previously cached Shell model
configuration. Empty environment values do not erase a non-empty cached value;
use the interactive `/config` flow or an explicit command-line value when you
intend to change stored Session configuration.

```bash
timem --help
timem-web --help
```

## Model Service Examples

Aliyun DashScope compatible mode:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://dashscope.aliyuncs.com/compatible-mode/v1
export TIMEM_MODEL=qwen-plus
export TIMEM_RESPONSE_PROTOCOL=xml
export TIMEM_MAX_LLM_INPUT=100K
export TIMEM_MAX_LLM_OUTPUT=20K
```

OpenAI:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=openai-responses
export TIMEM_BASE_URL=https://api.openai.com/v1
export TIMEM_MODEL=...
```

Anthropic:

```bash
export TIMEM_API_KEY=...
export TIMEM_API_PROTOCOL=anthropic
export TIMEM_BASE_URL=https://api.anthropic.com
export TIMEM_MODEL=...
```

Compatible or self-hosted service:

```bash
export TIMEM_API_PROTOCOL=openai-compatible
export TIMEM_BASE_URL=https://your-gateway.example/v1
export TIMEM_API_KEY=...
export TIMEM_MODEL=...
```

`TIMEM_API_PROTOCOL` chooses the model API wire format:

- `openai-compatible`
- `openai-responses`
- `anthropic`

`TIMEM_TOOL_CALL_MODE` chooses `auto`, `native`, or `inline` (default `auto`).
Auto mode probes the configured gateway/model and falls back to inline when
native tool calls are unsupported. `TIMEM_PARALLEL_TOOL_CALLS` accepts `auto`,
`true`, or `false`; Timem sends the resolved parallel flag explicitly to the API.

`TIMEM_RESPONSE_PROTOCOL` chooses the inline response format parsed by the local
runtime. Supported values are `xml` and `json`; default is `xml`. Native mode
uses provider tool-call structures, does not inject this inline protocol, and
automatically uses JSON prompt serialization. The configured inline protocol is
restored if the runtime later switches back to inline mode.

## Runtime Options

Common values:

```bash
export TIMEM_SPACE=/absolute/path/to/mem
export TIMEM_BASH_APPROVAL=approve
export TIMEM_WORK_INSTRUCTIONS=silent
```

`TIMEM_WORK_INSTRUCTIONS` controls `AGENTS.md` / `CLAUDE.md` loading:

- `silent`: auto-load and notify
- `ask`: ask the host UI
- `off`: do not load

`TIMEM_BASH_APPROVAL` controls model-requested command approval:

- `ask`: prompt before risky/local command execution
- `approve`: approve by policy for the current host; this is the default when unset

## Runtime Data

By default, Timem stores MEM data in the user's home directory:

```text
~/.timem/mem/
  audit/api_audit.json
  audit/api_audit.jsonl
  audit/action_audit.json
  audit/api_output_repair.json
  memory.jsonl
  scratch_notes.jsonl
  sessions/
    <session-id>/
      session.json
      raw_chat_history.jsonl
  session_groups.json
  worker_roles.json
  web_instance.json
  shell_history.txt
```

The directory is created automatically on first startup. Each MEM stores independent
capacity settings for temporary data, conversations, and favorites. Normal launch
defaults are 5 days plus 128 MiB for temporary data, 128 MiB for conversations, and
256 MiB for favorites. `--debug` changes only the absent launch defaults: audit becomes
512 MiB and temporary data becomes 512 MiB; conversations remain 128 MiB and favorites
remain 256 MiB. Explicit values already saved in the MEM, including unlimited values,
always win over launch defaults.

After Timem Web has published readiness, it applies temporary-data age and capacity
limits in a background blocking-file task; it runs again after a MEM switch and once
per hour while Timem Web is running. Ordinary chat-history appends do not wake this
global retention task. Conversation capacity is enforced by the same periodic task.
A direct setting change applies the new limit before reporting success. This keeps
large audit/history work off the listener-startup and chat-append paths. Temporary-data
age cleanup covers:

- raw-chat event kinds `action`, `action_result`, `context_compact`, and `repair`;
- finished shell-job records and their stdout/stderr/status files;
- API audit events in `audit/api_audit.json` and its JSONL sidecar.

User/assistant messages and all other history event kinds are never removed by this
setting. Running shell jobs are retained regardless of age. Unlimited mode skips the
time-based cleanup, but audit storage remains capacity-bounded. Cleanup uses the same
MEM lock domains as writers, retains records exactly on the cutoff boundary, and is safe
to repeat. Snapshot JSON files are replaced atomically. Large API-audit JSONL storage
uses physical 16 MiB segment files plus a small validated per-segment time/count summary.
Appending touches only the active segment; capacity cleanup unlinks complete oldest
segments; age cleanup unlinks wholly expired segments and rewrites only a segment whose
time range crosses the cutoff. Event timestamps need not be ordered inside a segment.

All bounded stores reserve one allocation slice for safe replacement and evict only
complete records or business items. Audit uses 16 MiB slices: normal launch has a
64 MiB hard bound and 48 MiB stable budget, while `--debug` has a 512 MiB hard bound
and 496 MiB stable budget. The API snapshot, action audit, and repair-output audit are
each individually bounded to one 16 MiB slice; the segmented API JSONL stream uses the
remaining directory budget (16 MiB normally and 464 MiB in debug mode) and retains the
newest complete segments/lines. A single API event over
16 MiB is replaced by a metadata-only `payload_omitted` record so one event cannot defeat the
bound. Conversations evict oldest complete turns, temporary capacity evicts oldest
complete temporary items, and favorites use physical 4 MiB segment files while keeping
complete favorite records. Capacity compaction preserves existing JSON/JSONL schemas.

Existing MEM directories require no manual migration. Upgrade readers recognize legacy
API-audit JSON documents, JSONL files at both `audit/api_audit.jsonl` and the older
MEM-root path, action-audit documents containing multiple Turns, segmented stores without
a manifest, and Session histories without an index or retention summary. The first
locked write or low-frequency maintenance pass builds the new manifest, per-Turn files,
or retention summary. Directory-based migrations are prepared in a sibling temporary
directory and installed by rename; legacy source data is removed or reduced only after
the new representation is committed, so an interrupted migration is safe to retry.
Malformed or missing small state files are rebuilt from retained records, while normal
appends validate state with file length or active-segment metadata and remain incremental.

This is an **upgrade compatibility** guarantee, not a bidirectional on-disk format
guarantee. Older Timem binaries do not understand every manifest, segment directory, or
summary introduced by newer releases. Keeping a complete legacy snapshot synchronized on
every append would reintroduce unbounded rewrites, so downgrade-in-place is unsupported.
Back up or export the MEM before downgrading, and never let old and new Timem versions
write the same MEM concurrently. Obsolete `.retention.tmp-*` audit copies left by older
versions are removed while the audit lock is held. Do not manually rewrite or delete a
live audit file from another process; restart the Timem host on the new version and let
its locked writer converge the store.

Session metadata is stored per Session rather than in one shared mutable index. If one
`sessions/<session-id>/session.json` is malformed or carries a mismatched ID,
Timem quarantines that file beside the Session directory and restores the other
healthy Sessions. Existing `sessions/index.jsonl` stores are repaired if
necessary, migrated automatically
to `sessions/<session-id>/session.json`, and retained as `index.v1.jsonl` for
manual recovery.

Writes use narrow lock domains: each Session owns one data lock for its metadata
and history; Session groups, Roles, tool jobs, and audit data use separate locks.
Independent Sessions therefore do not wait on each other's metadata or chat
writes. Cross-collection group changes release in-memory locks before filesystem or
worker operations. Group moves persist a candidate Session snapshot before
committing memory. Group deletion persists affected Session metadata first,
rolls those writes back if a later write fails, and removes the group definition
only after all Session writes succeed.

`--space` and `TIMEM_SPACE` select another MEM directory. Their value must be
an absolute path, and the path itself is the MEM directory; Timem does not add
an extra `memory` component:

On Unix, Timem creates the selected MEM directory with owner-only `0700`
permissions and tightens an existing selected MEM directory to `0700` at
startup. The parent directory is not modified. This protects Session runtime
configuration, cached credentials, memory, audit data, Web instance metadata, and Web
lifecycle diagnostics on multi-user Linux and macOS hosts.

```bash
timem --space /absolute/path/to/project-mem
export TIMEM_SPACE=/absolute/path/to/project-mem
```

Relative paths such as `--space .test_mem` are rejected.

When `timem-web` resolves the MEM directory to the system default
`~/.timem/mem` and no `--port` is supplied, it tries port `13764` first. If that
port is unavailable, it continues through the existing automatic port range
(`12345`–`23456`). A custom MEM uses the rotating automatic selection order,
and an explicit `--port` always has priority over either automatic strategy.

The selected MEM is the complete workspace root. Workspace configuration,
capability overlays, Sessions, memory, audit files, and Web diagnostics all live
under that one directory. `TIMEM_DATA_DIR` and `--data-dir` are no longer
supported and are rejected rather than silently splitting state across roots.
Existing project-local `.timem_data`, `data`, and other legacy directories are
not migrated or deleted automatically. Env files are independent from MEM data
and are not touched by install or uninstall scripts.

A MEM is exclusively owned by one running Timem host. A second Web or Shell
process using the same MEM exits with an ownership error; choose another
absolute `--space` to run concurrently. Different MEM directories remain
independent and may run at the same time.

### Timem Web lifecycle diagnostics

`timem-web` enables a small process-lifecycle recorder by default. Its purpose is
to preserve evidence for a later investigation when the Web host exits
unexpectedly, without continuously logging model or browser traffic.

Files are stored under:

```text
<MEM>/diagnostics/timem-web/
  current-runs/
    <run-id>.json
  run-archive/
    <run-id>-exit.json
    <run-id>-panic.txt
    <run-id>-abnormal.json
  last-exit.json
  last-panic.txt
  previous-abnormal-exit.json
```

The recorder has fixed bounds:

- at most 64 recent lifecycle events are kept in memory;
- disk checkpoints occur only at process start, configuration completion,
  listener binding, graceful exit, or panic;
- files are atomically replaced rather than appended, so storage does not grow
  with uptime or request count; completed-run archives retain only a bounded
  recent set;
- panic messages are limited to 4 KiB and backtraces to 128 KiB;
- Unix directories and files use owner-only `0700` and `0600` permissions.

Each `current-runs/<run-id>.json` contains the process version, PID,
PID-reuse-resistant process identity where supported, operating system,
architecture, option names, and the latest low-frequency milestone. Successive
Web runs use separate files. A process removes only its own marker, and a new
process promotes only markers whose owner is confirmed stale. Argument
values are not stored. In particular, API keys, URLs, paths, prompts, model
replies, Web access tokens, HTTP header values, and tool inputs/outputs are not
part of the default lifecycle report.

On a graceful exit, `last-exit.json` records the actual selected trigger:
`ctrl_c`, `sigterm`, `sighup`, `parent_process_exited`, `server_completed`, or
`help_requested`. It also records whether runtime cleanup completed. A startup
or runtime error is recorded separately as `startup_or_runtime_error`, with a
bounded best-effort redaction of common credential shapes.

A Rust panic produces `last-panic.txt` with its thread, source location, recent
lifecycle milestones, and a forced backtrace. Backtrace capture occurs only on
panic. If the process cannot run cleanup—for example after SIGKILL, host reboot,
or some OOM terminations—its file under `current-runs/` remains. A later start
copies a confirmed-stale record to `run-archive/<run-id>-abnormal.json` and
updates `previous-abnormal-exit.json` with `exact_cause: unknown`. This proves
only that the prior process did not complete its exit protocol; it does **not**
by itself prove a panic, OOM, or any particular external signal.

For an unexpected exit, preserve these files before reproducing again. Start
with `last-exit.json`, `last-panic.txt`, and `previous-abnormal-exit.json`.
Review their contents before sharing them. The separate `--debug` mode remains
opt-in and may contain prompts, model replies, and tool data; it is intended for
model-interaction diagnosis rather than ordinary process-exit diagnosis.

## Interactive Notes

Shell:

- `/help` lists runtime commands.
- `/config` changes runtime settings in the current process.
- `/prof` shows runtime profiling.
- `/workspace` manages workspace reference directories.
- `Ctrl+C` / `Esc` cancel the current input/menu/turn; use `/exit` or `Ctrl+D`
  to exit.
- While the model is working, typing another question and pressing Enter queues a
  separate next turn; it does not replace the current turn’s final answer.

Web:

- When creating a Session, choose a registered Workspace or enter an existing
  absolute directory on the Timem host. The selected directory becomes that
  Session's CWD.
- Sessions can use different model/API/runtime settings.
- The sidebar supports persistent Session groups. Groups can be created,
  renamed, reordered, collapsed, and deleted; deleting a group moves its
  Sessions to **Unsorted** without deleting them. A Session can be moved between
  groups while other Sessions continue working.
- Attachments are stored under the active MEM and passed to the active
  turn.
- Stop cancels all workers in the active Session; the next send starts from the
  primary worker.
- History is restored in pages so long conversations do not block the UI.

## Update

```bash
git pull --ff-only
./install.sh
```

## Uninstall

```bash
./uninstall.sh
```

Uninstall removes the binaries and installed reminder resource. It does not remove user configuration, including a user-level `reminder_tips.json` override.
If Rust was installed only for Timem, remove it separately:

```bash
rustup self uninstall
```

## MEM 历史记录保留

Timem Web 左下角的 **Memory** 卡片用于打开当前 MEM 的设置；卡片右侧的独立切换按钮仅用于切换 MEM 目录。

每个 MEM 可以单独设置 Session 聊天历史的保留期限：

- 最近 1 天
- 最近 5 天（默认）
- 最近 10 天
- 不限

设置保存在当前 MEM 的 `mem_settings.json` 中。用户修改为有限期限时，Timem 会先应用新期限再报告成功。Timem Web 启动时会先完成端口监听并报告 ready，再在后台应用该策略；切换到另一个 MEM 后也会在后台立即应用，运行期间每小时再执行一次。这样大体量历史或审计文件的扫描与原子重写不会阻塞 Web 启动。该操作不会删除 Session、ToolRepo 工具、角色、MCP 或模型接入点。为避免与正在写入的历史冲突，存在运行中任务时不能修改保留期限。
