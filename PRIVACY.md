# Privacy

Agent Monitor HUD is local-first. It has no telemetry, no account and no server of its own.

## Data read

Only the bounded metadata needed to show what is running now:

- **Codex** (`~/.codex`, and `~/.codex-deepseek` if you use that profile): `token_count` counters
  and rate-limit percentages, lifecycle events (`task_started`, `task_complete`, `turn_aborted`),
  `turn_context` (model label and the leaf folder name of the working directory), and
  `session_index.jsonl` (`id`, `thread_name`).
- **Claude Code** (`~/.claude/projects`): session id, entry point, working-directory leaf, title
  records and per-message token usage. Hook events report lifecycle only.
  **Claude Desktop** plan usage: the 5-hour and weekly percentages it already stores locally.
- **DeepSeek Harness** (`~/.dsh`): its session projection cache (title, model, token usage,
  context pressure, turn and question state). The compressed conversation log is never opened.

Prompts, replies, tool output and attachments are never read for display, stored, or sent
anywhere. Provider configuration and authentication files are not read.

## Data stored

In `%LOCALAPPDATA%\CodexMonitorHUD`: your settings, a task registry (task number, workspace leaf,
coarse status, update time), a per-day activity ledger (token counts per minute), and a log.

If you enter a DeepSeek API key, it is encrypted with Windows DPAPI for your user account and
stored in `secrets\` — not in `settings.json`, logs, previews or the clipboard. Removing it in
Settings deletes the file.

## Network

- Optional: with a DeepSeek API key saved, the HUD calls `https://api.deepseek.com/user/balance`
  to show your balance. Nothing else is sent, and no request is made without a key.
- Codex allowance: the HUD may ask the already signed-in local Codex client for the 5-hour and
  weekly percentages; the HUD itself does not handle Codex credentials.

## Claude Code integration

`integrate claude` adds the HUD's lifecycle hooks and a user-scope MCP server to Claude Code's own
config files; `unintegrate claude` (and the uninstall/rollback scripts) removes exactly those
entries and nothing else.

## Sharing

Do not attach local logs, session files, databases, settings or keys when reporting a bug. A
screenshot or a `render-preview` image is usually enough.
