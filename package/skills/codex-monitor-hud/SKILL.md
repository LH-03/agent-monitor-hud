---
name: codex-monitor-hud
description: Operate, configure, explain, troubleshoot, or send opt-in task-targeted notices through the locally installed Agent Monitor HUD overlay (Codex, Claude Code and DeepSeek Harness tasks). Use when the user mentions HUD, Codex Monitor, "收尾保护", "额度保护", "额度快没了", or asks to preserve a recoverable handoff.
---

# Agent Monitor HUD

Use the plugin's MCP tools for normal operation:

- `monitor_hud_open_settings` opens the appearance and metric settings window.
- `monitor_hud_show` shows or restarts the HUD.
- `monitor_hud_hide` hides it without deleting settings.
- `monitor_hud_pause` toggles live updates.
- `monitor_hud_status` reports runtime health, configured source filters, and privacy-safe active task source metadata (including each task's `agent`: `codex`, `claude` or `dsh`).
- `monitor_hud_quota_guard` reports the latest locally observed Codex 5-hour/weekly allowance and a conservative handoff advisory.
- `monitor_hud_notification_capabilities` reports whether proactive notices are off, text-only, or allowed to use bounded live choreography. It also lists privacy-safe active HUD task numbers.
- `monitor_hud_notify` sends one short notice, labeled with the calling agent (CODEX, CLAUDE or DEEPSEEK), to the matching task surface without ending or pausing the current turn.

## Proactive notice protocol

Do not rely on memory from another conversation. Tools are rediscovered when the plugin loads in each new task; an already-open task may need to be recreated after a plugin update.

Before the first proactive notice in a task:

1. Call `monitor_hud_notification_capabilities`.
2. If notices are disabled, continue normally and do not repeatedly retry.
3. Match the current workspace and source to an active task number and pass `task_number`. If two visible tasks are ambiguous, ask the user instead of guessing. Omitting the number falls back to the most recently active task of the same agent.
4. Follow the user's or developer's notification policy. Useful mid-turn cases include a decision that needs human review, a blocked external step, a risky action awaiting approval, or a milestone the user explicitly asked to watch. Do not notify for routine progress unless instructed.
5. Keep the message to one or two plain-text sentences, at most 160 characters. Never include secrets, full logs, private prompts, links, or rich text.

A notification is a side-channel attention cue. After the tool returns, keep working unless the underlying task genuinely requires user input. It must not change task status, manufacture a completion event, or replace the normal final answer.

With `text` permission, send only the message and task number; the user's configured style is used. With `expressive` permission, the agent may compose a bounded recipe from `glow`, `pulse`, `breathe`, and `flow`, plus color, intensity, tempo, cycles, glow radius, scale, and direction. This is declarative visual data, never executable code. Use motion semantically and avoid continuous distraction.

## Opt-in allowance handoff guard

When the user says **“HUD 收尾保护”**, **“额度收尾保护”**, or equivalent natural language, immediately call `monitor_hud_quota_guard` once, then follow it at natural checkpoints and before a broad, expensive next step. It returns only the latest locally observed Codex account-level 5-hour/weekly percentages; Claude Code and DeepSeek Harness have no local allowance data, so unavailable data must stay unavailable rather than being guessed.

The user enables it and chooses the four editable thresholds in **Settings > Multi-task > Allowance handoff guard**. If the tool returns `disabled`, say that the protection is off and offer to open Settings; do not silently alter settings.

- `clear`: continue normally.
- `prepare_handoff`: at the next checkpoint, prepare a concise recoverable handoff before expanding scope.
- `handoff_now`: before an expensive next step, write/update a concise handoff with current state, changed files, verification, and the exact next command; then stop expanding scope.

For either low state, follow the tool's `instruction` field as the user's current template. First look for an existing project handoff document or prescribed project format and update that when present; create a concise new handoff only when the project has none.

`should_alert` is one-shot: it becomes true when the observed allowance first enters a low band or escalates from preparation to critical. When it is false with `event: steady`, do not repeatedly rewrite or announce the same handoff merely because the allowance remains low.

This is an advisory read tool, not a remote interrupt. Do not manufacture a completion or stop a task solely because the HUD reports a low allowance.

For creating a shareable skin, use `$create-monitor-hud-theme`. Finished `.json`, `.cmhud-theme`, and safe `.cmhud-theme.zip` files install from **Settings > General > Theme workshop**.

## Accounting rules

- Cached input is a subset of input; fresh input is input minus cached input.
- Call total is input plus output. Reasoning output is already inside output and is never added again.
- Codex task totals come from `total_token_usage.total_tokens`.
- Claude Code input is `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`; cached is the cache-read part. Responses are de-duplicated by message id, and the context window is estimated from the model name (shown with ≈).
- DeepSeek Harness totals and context pressure come from the harness's own token-usage projection.

## Privacy and safety

The HUD reads only metadata and token counts: Codex session identity, lifecycle, `token_count` and `turn_context` records (`~/.codex`, optionally `~/.codex-deepseek`); Claude Code project transcripts under `~/.claude/projects` (usage, model, titles and turn boundaries — never prompt or reply text); and the DeepSeek Harness session projection cache under `~/.dsh/storages`. Claude hooks only report lifecycle events. It never reads provider configuration or authentication files, uploads logs, stores prompts, or modifies sessions. Settings, the privacy-safe task-number registry and queued notices live under `%LOCALAPPDATA%\CodexMonitorHUD`.

All sources share one bounded monitor and stable-number pool. Closing a detached bubble only merges that surface back into the main HUD; it does not dismiss the task from aggregate monitoring.

When troubleshooting, run `bin\AgentMonitorHUD.exe dump` from the plugin folder: it prints the visible task table without opening any window. `%LOCALAPPDATA%\CodexMonitorHUD\hud.log` holds a bounded runtime log. Do not delete session logs or the user's usage database.
