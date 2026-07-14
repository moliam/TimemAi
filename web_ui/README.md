# Timem Web UI

The browser client uses assistant-ui for the conversation surface and is
embedded into the `timem-web` Rust binary from `timem-web/dist`.

Run the frontend checks and rebuild the tracked production assets after any UI
change:

```bash
pnpm --dir web_ui/timem-web install --frozen-lockfile
pnpm --dir web_ui/timem-web test
pnpm --dir web_ui/timem-web build
cargo test -p timem_web
```

Do not commit `node_modules` or the optional upstream source checkout under
`web_ui/vendor`. Commit the lockfile, application source, tests, and rebuilt
`dist` assets together. Read `module_boundary.md` before changing host/core
responsibilities.
