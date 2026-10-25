# Architecture

Agent Monitor HUD 4.x is a single native Windows executable written in Rust. It replaced the
3.x C#/WPF application (plus its Node.js MCP host and bundled .NET runtime) while keeping the
same look, settings file and plugin identity. The rewrite was done by Claude Opus 5.5 working
in Claude Code; the 3.x source stays in this repository's history as the behavioural reference.

## Measured footprint

Migration records on the same Windows desktop show resident private memory changing from about
176 MiB to 12–13 MiB (about 93% lower), and working set from about 177 MiB to 30 MiB (about 83%
lower). The complete v4.0.0 release unpacks to 3.77 MB, including its documentation and screenshots.
See [PERFORMANCE.md](PERFORMANCE.md) for dates, process boundaries, CPU normalization and exact
artifact sizes.

## Process model

One executable, `AgentMonitorHUD.exe` (no .NET, Node.js, PowerShell or WebView at runtime):

| Mode | What it does |
| --- | --- |
| *(default)* | The HUD. Single instance (`Local\AgentMonitorHUD` mutex); a second launch forwards its request (`--open-settings`, `show`, …) to the running instance and exits. |
| `mcp` | STDIO MCP server for Codex and Claude Code. Tiny, no GUI; starts the HUD if it is not running. |
| `hook` | Claude Code hook handler: reads the hook JSON on stdin, writes a whitelisted state file, starts the HUD on `SessionStart` / `UserPromptSubmit`. Never blocks or injects into Claude. |
| `integrate claude` / `unintegrate claude` | Adds or removes only the HUD's own hooks and user-scope MCP entry. |
| `render-preview <dir> [scenes]` | Off-screen PNG renders of synthetic states (visual regression checks). |
| `dump` | Prints the engine's current view as JSON (diagnostics). |

Threads in the HUD: the UI thread (windows, layout, Direct2D rendering), one watcher thread
(`ReadDirectoryChangesW`) and one engine thread (discovery, incremental parsing, lifecycle). The
engine publishes immutable snapshots to the UI.

## Rendering

Layered windows (`UpdateLayeredWindow`, per-pixel alpha) drawn with a Direct2D software render
target and DirectWrite. A small WPF-like layout tree (Border / Stack / Wrap / Grid / Text /
Ellipse / Icon / Bar / Sparkline) reproduces the XAML measure/arrange rules, so sizes and paddings
match the 3.x windows. The settings window and color picker are drawn by the same engine. No GPU
device is created.

The main bubble's corner drag only loosens spacing (gaps, padding, list row spacing) between a
tight and a fully spread layout; type never scales and the bubble never reflows into a box.

## Sources

### Codex

- `~/.codex/sessions/**.jsonl` (and the optional isolated `~/.codex-deepseek` profile): only
  `token_count`, lifecycle events and `turn_context` (model, workspace leaf) are parsed; titles come
  from `session_index.jsonl`.
- The 5-hour and weekly allowance comes from `token_count.rate_limits` in the session logs. The
  official local `codex app-server` read runs only at startup and at most every 10 minutes while a
  Codex task is visible.
- Incremental per-file offsets plus a recursive directory watch; a full scan only at start, on
  watcher overflow and every 5 minutes.

### Claude Code

- `<CLAUDE_CONFIG_DIR or ~/.claude>/projects/<project>/<sessionId>.jsonl`; subagent files never
  become top-level tasks.
- Identity: `sessionId`, `entrypoint` (desktop Code tab or CLI), `cwd` leaf; title from
  `custom-title` / `ai-title` / `agent-name` records only (never message text).
- Usage per assistant message keyed by `message.id` (+ `requestId`): later writes of the same
  response replace earlier values. Context % = input total / model window.
- Lifecycle from hooks (`SessionStart`, `UserPromptSubmit`, `Notification`, `Stop`, `StopFailure`,
  `SessionEnd`), with transcript growth as the fallback. `waiting` means Claude needs your approval
  or input.
- Plan usage (5-hour and weekly %) is read from the usage samples Claude Desktop already keeps
  locally (`plan-usage-history.json`, about every 15 minutes).

### DeepSeek Harness

- `<DSH_HOME or ~/.dsh>/storages/session_projcache/sessions/<session>.json`: the harness's own
  throttled checkpoint of its session projections. The compressed conversation log is never
  opened.
- Fields used: title, working directory, model, token usage, context pressure, open turn, open
  user question, last message time.
- Account balance (optional): only when you enter your own DeepSeek API key in Settings → Agents.
  The key is encrypted with Windows DPAPI for the current user, stored apart from
  `settings.json`, and sent only to `api.deepseek.com/user/balance` (every 10 minutes while
  DeepSeek Harness was active in the last hour, otherwise hourly).

## State

Everything lives in `%LOCALAPPDATA%\CodexMonitorHUD` (the 3.x folder name is kept so settings carry
over): `settings.json`, the task registry, a per-day activity ledger (token counts per minute for
the rhythm sparkline), `secrets\` (the DPAPI-protected key, if any) and `hud.log`.

## Install layout

`scripts\install.ps1` installs to `~/plugins/codex-monitor-hud` (the plugin id is unchanged), adds
the personal marketplace entry Codex uses, refreshes Codex's plugin cache, creates the settings
shortcuts and, when Claude Code is present, runs `integrate claude`. Coming from 3.x, the old
plugin folder is kept whole as `~/plugins/.codex-monitor-hud-rollback-<version>` and
`scripts\rollback.ps1` restores it. While a 3.x Node MCP server is still alive in an open Codex
session, the native HUD keeps the legacy heartbeat fresh so that server never relaunches 3.x.

## Fixed on purpose (compared with 3.x)

| Old behaviour | New behaviour |
| --- | --- |
| Old conversations could stay listed forever (Codex keeps rollout files open) | A locked file is only a liveness hint while a turn is open; no "latest file of all time" fallback. |
| Allowance read spawned `codex app-server` every 20 s | Read from session logs; the app-server read is rare (see above). |
| Whole `sessions` tree enumerated every 30 s | Directory watch + incremental offsets. |
| Stale `hosts/*.heartbeat` files piled up | Hosts register PID + process start time; stale entries are removed. |
| Settings window sometimes never appeared | Settings is an in-process window of the HUD. |
| Color picker could only pick saturated colors or white | Real HSV wheel. |
| Font size only grew some row text | Row and bubble typography, badges and icons scale together. |
