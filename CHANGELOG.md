# Changelog

<!-- release:start -->
## 0.1.0 (2026-05-25)

First public release.

- Markdown-as-protocol projection of a real Chromium browser for AI agents.
- Per-tab page files, diff files, console.md, screenshots, recordings.
- Sessions and Chrome profile reuse (default, named, no-profile).
- Action grammar: goto, click, fill, type, press, scroll, wait_text, wait_url, wait_idle, back, reload, screenshot, record_start, record_stop.
- Patches against upstream agent-browser:
  - `--no-activate` for background tab switching.
  - `--in-place` recording (no new context).
  - Per-tab console + uncaught-exception buffer.
<!-- release:end -->
