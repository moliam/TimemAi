# TimemAi 1.1.3

TimemAi 1.1.3 gives the recommended Timem Web interface a consistent browser
identity. Install TimemAi, run `timem-web`, and configure each Session directly
in the browser.

## Highlights

- **Timem in the browser tab:** the Web page now uses the same Timem logo shown
  in the application sidebar, making local and remote Timem tabs easy to find.
- **Included in the released binary:** the favicon is part of the committed
  production Web bundle embedded in `timem-web`; Node.js is not required after
  installation.
- **Simple Web startup:** run `timem-web` for authenticated local use, or
  `timem-web --public` when remote access is explicitly intended. Configure the
  selected Session's API key, model, protocol, and Base URL in the Web UI.
- **Release-quality coverage:** frontend contracts verify the favicon source,
  and Rust Host tests verify that the embedded PNG exists and is served with the
  browser-safe content type. The complete production gate runs on macOS and
  Linux before publication.

## Upgrade

```bash
git pull --ff-only
./install.sh
timem-web
```

Open the complete authenticated URL printed by `timem-web`, including its
`?token=...` value. Closing a page does not invalidate that URL while the Host
continues running.
