# TimemAi 1.3.0

TimemAi 1.3.0 focuses on a faster, smoother, and more dependable everyday experience—especially for long conversations, parallel work, and Sessions that stay active for extended periods.

## Highlights

### Faster and lighter

- Long conversations scroll more smoothly, and switching between Sessions feels faster and more stable.
- Frequent background updates now use fewer disk and system resources, reducing overhead during long-running work.
- Temporary data is managed more efficiently, with clearer controls for reviewing and limiting its storage use.
- Large command output and extensive history remain responsive without unnecessary repeated processing.

### More reliable stopping and follow-up

- Stopping a task is more dependable, with clearer status and fewer stale updates after cancellation.
- You can prepare the next question while the current task is stopping, so work can continue with less waiting and fewer interruptions.
- After a restart, interrupted work is shown accurately instead of appearing to still be active.
- Long-running commands and temporary results are handled more consistently across Sessions.

### A better reading and research experience

- Long answers now include a compact heading navigator, making it easier to jump between sections and keep your place.
- Chat search and favorites make important conversations easier to find and revisit.
- Useful partial answers can appear before the final response, while the finished conversation remains clean and easy to read.
- Search results, Session lists, settings, and activity panels have been refined for clearer and more efficient navigation.

### More polished Timem Web

- Session switching, live progress, scrolling, and message navigation are smoother during active work.
- Stop controls and cancellation feedback are clearer and more consistent.
- Polling and command activity provide more useful at-a-glance progress information.
- Local and public access retain their existing safety boundaries, with public access continuing to require authentication.

## Upgrade

```bash
git pull --ff-only
./install.sh
timem-web
```

Existing MEMs and Sessions remain available after upgrading. If Timem was restarted during an active task, that task will be marked as interrupted so you can decide whether to continue or start again.
