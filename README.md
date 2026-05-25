# preprint

> An experiment in projecting the live web as a filesystem so AI agents can drive a real browser by reading and writing markdown.

The web is the largest live source of structured + unstructured state we have, but it's locked behind a rendering engine. Agents that need to act on the web today either learn a thick automation protocol (CDP, Playwright, Puppeteer) or read flattened snapshots that lose interactivity and freshness.

**preprint takes a different bet.** A daemon owns a real Chromium instance. Every open tab is *projected* as a markdown file you can read with `cat`. To act, the agent appends exactly one line under a marker. The daemon executes it against the browser and rewrites the file to reflect the new state: accessibility tree, URL, last action, console output, all live.

The interface the agent sees is the one it already knows: read a file, append a line. The interface the browser receives is high-fidelity CDP. The markdown sits between them as the contract.

---

## Install

```sh
npm install -g @supermemory/preprint
```

Or with `npx`:

```sh
npx @supermemory/preprint open https://example.com --context "demo"
```

First run downloads no extra runtime. Chrome is auto-detected from your system; if missing, the underlying agent-browser binary will tell you how to install it.

## Quick start

```sh
# Open a real Chrome window (your default profile, default session)
preprint open https://news.ycombinator.com --context "scan today's frontpage"

# See what's there
ls preprint/
cat preprint/tabs.md

# Read the live page projection
cat preprint/news.ycombinator.com-t1.default.md

# Drive it. Append one action under the marker
echo 'click(@e3)' >> preprint/news.ycombinator.com-t1.default.md

# Within ~1 second the file is rewritten; check the result
grep -A1 "## Last Action" preprint/news.ycombinator.com-t1.default.md

# Close when done
preprint close news.ycombinator.com-t1.default
```

That's the whole loop: read the file, append one action, re-read.

## How it works

When `preprint open` runs, three things happen:

1. A background **preprint daemon** starts (or reuses one).
2. The daemon launches **agent-browser**, which controls Chromium via CDP.
3. preprint creates `preprint/<tab_key>.md` and `preprint/tabs.md` in your workspace, and starts polling.

Every poll cycle (~750ms by default) the daemon:

- Snapshots the page's accessibility tree, normalises it, writes it under `## Page`.
- Reads any action appended below `<!-- preprint:actions -->`.
- Executes the action against the live browser.
- Drains the page's console + uncaught exceptions into `.preprint/artifacts/<tab_key>/console.md`.
- Rewrites the page file with the new state and result.

The markdown file is the source of truth for the agent. The browser is the source of truth for the world. The daemon keeps them in sync.

## Files

```
preprint/                                              # the projection (read these)
  tabs.md                                              # every open tab; reuse before opening duplicates
  <host>-tN.<session>.md                               # per-tab live page
  <host>-tN.<session>.diff.md                          # what changed since the previous snapshot

.preprint/                                             # daemon state (no need to read directly)
  daemon.pid
  daemon.log                                           # only populated with --dev
  artifacts/
    <host>-tN.<session>/
      session.json                                     # daemon's view of this tab
      console.md                                       # live page console + JS exceptions (rolling 500 lines)
      screenshots/<name>.png                           # output of screenshot() actions
      recordings/<name>.webm                           # output of record_start / record_stop
```

`<host>` is the tab's initial host (`gmail.com`, …). `tN` is the stable tab id (`t1`, `t2`, …). `<session>` is the agent-browser session (`default` unless `--session` was passed). The three together form a unique **tab_key** that names every file related to that tab.

## Action grammar

One action per append. Anything below `<!-- preprint:actions -->` is consumed by the next poll.

```
goto("https://example.com")        navigate the tab to a URL
snapshot()                         force a fresh snapshot (rare; daemon does this)
click(@ref)                        click an interactive element (ref from `## Page`)
fill(@ref, "text")                 clear + type into an input
type(@ref, "text")                 type into an input without clearing
press("Enter")                     press a key; modifiers ok ("Control+a")
wait_text("Done")                  wait for visible text on the page
wait_url("**/dashboard")           wait for the URL to match a glob
wait_idle()                        wait for the network to go idle
scroll("down", 500)                scroll N px (up | down | left | right)
back()                             browser back
reload()                           reload page
screenshot()                       capture PNG; path reported in last_action
screenshot("login")                named PNG (overwrites if name exists)
screenshot("login", annotate)      same + draws [N] boxes for @e1, @e2, …
record_start("demo")               begin video; header shows "Recording active: demo (path)"
record_stop()                      end recording; .webm path in last_action
```

Refs (`@e1`, `@e2`, …) come from the `## Page` section of the current snapshot and **renumber every snapshot**. Re-read the page file before every action.

## Sessions and profiles

A *session* is one Chromium instance with its own cookies, storage, and identity. Multiple tabs can share one session.

```sh
preprint open <url> --context "..."                          # default Chrome profile, session "default"
preprint open <url> --context "..." --profile "Work"         # named Chrome profile, its own session
preprint open <url> --context "..." --no-profile             # clean Chromium, no identity, session "no-profile"
preprint open <url> --context "..." --session <name>         # explicit session name
preprint open <url> --context "..." --preview                # also show the browser window (headed)
```

Resolution:

- No flag → your default Chrome profile, `default` session.
- `--profile X` where X is your default → still `default` session (one Chromium for "your normal browser").
- `--profile X` where X is something else → its own auto-named session, separate Chromium.
- `--no-profile` → `no-profile` session, no identity.
- `--session <s>` always wins for naming.

A session's profile is **locked at creation**. To switch identities, close that session's tabs first or use a different `--session` name.

`--context "<one-line purpose>"` is required in practice. It's how the next agent (or future you) finds the right tab via `preprint/tabs.md`.

## Per-tab artifacts

Three sibling files under `.preprint/artifacts/<tab_key>/`:

- **`console.md`**: live tail of `console.log` / `warn` / `error` + uncaught JS exceptions for that tab. Rolling 500-line cap. Created on tab open, fills as the page emits.
- **`screenshots/<name>.png`**: saved screenshots. `screenshot()` auto-names; `screenshot("login")` uses the name.
- **`recordings/<name>.webm`**: saved video from `record_start("demo")` to `record_stop()`. While recording, the tab's header shows `Recording active: <name> (path)`.

Screenshots and recordings stay across tab close (they're artifacts). `preprint stop` sweeps the whole `.preprint/` tree.

## Commands

```sh
preprint open <url> [flags]      # open a tab (see Sessions and profiles above)
preprint close <tab_key>         # close one tab; last tab in a session tears down the session
preprint status                  # daemon + open-tabs summary
preprint stop                    # stop the daemon and all sessions, sweep .preprint/
preprint --dev <subcommand>      # enable daemon logs at .preprint/daemon.log
```

## Use with AI agents

The preprint daemon writes a Claude Code-compatible skill at `skills/preprint-browser/SKILL.md`. Add it to your agent so it loads the workflow automatically:

```sh
npx skills add supermemoryai/preprint
```

This works with Claude Code, Cursor, Codex, Gemini CLI, GitHub Copilot, Goose, and others that read the [skills.sh](https://skills.sh) format.

If you'd rather wire it manually, add this to your project's `CLAUDE.md` / `AGENTS.md`:

```markdown
## Browser

This project uses preprint to drive a real Chromium browser through markdown files.
- `ls preprint/` to see open tabs.
- `cat preprint/<tab_key>.md` to read a tab's live page projection.
- Append exactly ONE action under `<!-- preprint:actions -->` to act.
- The `## Last Action` line will say `ok …` or `error …` within ~1 second.
- Refs (`@e1`, `@e2`) come from `## Page` and renumber every snapshot, so always re-read.
- For console output, read `.preprint/artifacts/<tab_key>/console.md`.
```

## Install from source

preprint vendors patches against [vercel-labs/agent-browser](https://github.com/vercel-labs/agent-browser) (Apache-2.0). The patched binaries for all seven platforms ship inside the repo at `agent-browser/`, refreshed by a CI workflow (`agent-browser-binaries`) that applies [`patches/agent-browser/`](./patches/agent-browser) to a clean upstream checkout. To build preprint locally you don't need to touch any of that; the binaries are already there.

```sh
git clone https://github.com/supermemoryai/preprint
cd preprint
cargo build --release                          # self-contained, embeds agent-browser
cargo build --release --no-default-features    # sidecar mode (for the npm-style layout)
```

If you want to refresh the patched agent-browser binaries (after editing a patch or pulling upstream changes), trigger the `agent-browser-binaries` GitHub Actions workflow. It's the canonical source of those artifacts.

## Repository

- Code: [github.com/supermemoryai/preprint](https://github.com/supermemoryai/preprint)
- Issues: [github.com/supermemoryai/preprint/issues](https://github.com/supermemoryai/preprint/issues)

## License

Apache-2.0. See [LICENSE](./LICENSE).
