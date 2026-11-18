# Architecture

Agent Monitor HUD is a native Rust executable for Windows. The 3.x version used a WPF interface and a Node.js MCP host.

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
| `mcp` | STDIO MCP server for Codex. Tiny, no GUI; starts the HUD if it is not running. |
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
  official local `codex app-server` read runs every 30 seconds during activity and every 2 minutes idle.
- Incremental per-file offsets plus a recursive directory watch; a full scan only at start, on
  watcher overflow and every minute. Directory creation/moves also trigger discovery. The local
  activity index is queried every 2 seconds during activity and every 5 seconds idle, without relying
  on database/WAL file timestamps. Partially written new headers retain a forward reader at byte zero.

### DeepSeek Harness

- `<DSH_HOME or ~/.dsh>/storages/session_projcache/sessions/<session>.json`: the harness's own
  throttled checkpoint of its session projections. The compressed conversation log is never
  opened.
- Fields used: title, working directory, model, token usage, context pressure, open turn, open
  user question, last message time.
- Account balance (optional): only when you enter your own DeepSeek API key in Settings → Agents.
  The key is encrypted with Windows DPAPI for the current user, stored apart from
  `settings.json`, and sent only to `api.deepseek.com/user/balance` (about every 5 seconds active,
  every minute idle, with debounced reads after usage/turn changes and failure backoff).

## State

Everything lives in `%LOCALAPPDATA%\CodexMonitorHUD` (the 3.x folder name is kept so settings carry
over): `settings.json`, the task registry, a per-day activity ledger (token counts per minute for
the rhythm sparkline), `secrets\` (the DPAPI-protected key, if any) and `hud.log`.

## Install layout

`scripts\install.ps1` installs to `~/plugins/codex-monitor-hud` (the plugin id is unchanged), adds
the personal marketplace entry Codex uses, refreshes Codex's plugin cache, creates the settings
shortcuts. Coming from 3.x, the old
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
