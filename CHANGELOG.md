# Changelog

## 4.0.0 - Agent Monitor HUD: native Rust rewrite

Rewritten from scratch in Rust by Claude Opus 5.5 (Anthropic) in Claude Code, with the maintainer
directing the design. Renamed from Codex Monitor HUD because it now watches more than Codex.

- One native executable replaces the WPF app, its bundled .NET runtime and the Node.js MCP
  host. Settings, hook handling and the MCP server are native executable modes.
- New sources: **Claude Code** (desktop Code tab and CLI, with lifecycle hooks and a `waiting`
  state for approvals/questions) and **DeepSeek Harness** (desktop).
- Plan and balance: Claude 5-hour and weekly usage from Claude Desktop's local samples; optional
  DeepSeek API balance with a DPAPI-encrypted key you enter yourself.
- Main bubble: choose what leads (metrics, an activity-rhythm sparkline with today's total, or
  nothing), a compact task status, per-agent quota meters that warn when low, and a waiting badge.
  Dragging the corner loosens spacing within limits; type never scales.
- Settings redesigned with a sidebar and six pages (General, Appearance, Main Bubble, Tasks,
  Alerts, Agents).
- Softer attention glow on list rows; the task list sizes to its content.
- Updated task retention and watcher-driven incremental reads; stale MCP hosts are cleaned up.
- Settings now opens in the HUD process, and the color picker uses native HSV controls.
- Same settings folder (`%LOCALAPPDATA%\CodexMonitorHUD`), plugin id and themes. Installing over
  3.x keeps the whole old plugin folder for `scripts\rollback.ps1`; `scripts\uninstall.ps1` is new.
- The MCP server is now registered as `agent-monitor-hud`.

## 3.x and earlier

The Codex Monitor HUD changelog (1.0.0 – 3.3.0) is in this repository's history: see
[`CHANGELOG.md` at v3.3.0](https://github.com/LH-03/agent-monitor-hud/blob/v3.3.0/CHANGELOG.md).
