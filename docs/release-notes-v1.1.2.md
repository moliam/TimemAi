# TimemAi 1.1.2

TimemAi 1.1.2 hardens startup and installation for the recommended Timem Web
experience on macOS and Linux. Install TimemAi, run `timem-web`, and configure
each Session directly in the browser.

## Highlights

- **Start reliably:** recoverable event-journal, command-cache, MCP, and
  Session-index damage is backed up privately and repaired instead of making
  the Web Host unstartable.
- **Safe updates:** `./install.sh` replaces built executables atomically, so an
  update cannot truncate a running macOS or Linux binary.
- **Reusable local access:** closing the browser does not revoke the current
  authenticated URL. The same URL can open the running Host repeatedly; a Host
  restart intentionally prints a newly authenticated URL.
- **Simple Web configuration:** start with `timem-web`, click the model name,
  and configure the API key, model, protocol, and Base URL for that Session.
  Use `timem-web --public` only when remote access is explicitly intended.
- **Public release validation:** the same production gate runs on clean Ubuntu
  and macOS GitHub runners, including warning-free Rust builds, full Rust and
  Web tests, optimized performance guards, release builds, loopback lifecycle
  tests, and real pseudo-terminal smoke tests.

## Upgrade

```bash
git pull --ff-only
./install.sh
timem-web
```
