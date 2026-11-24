# Privacy

Agent Monitor HUD is local-first. It has no telemetry, no account and no server of its own.

## Data read

Only the bounded metadata needed to show what is running now:

- **Codex** (`~/.codex`, and `~/.codex-deepseek` if you use that profile): `token_count` counters
  and rate-limit percentages, lifecycle events (`task_started`, `task_complete`, `turn_aborted`),
  `turn_context` (model label and the leaf folder name of the working directory), and
  `session_index.jsonl` (`id`, `thread_name`).
- **DeepSeek Harness** (`~/.dsh`): its session projection cache (title, model, token usage,
  context pressure, turn and question state). The compressed conversation log is never opened.
- **Google Antigravity 2** (`~/.gemini/antigravity`): read-only conversation SQLite metadata
  (title, workspace, model, status, cached and uncached input, output, and model timing). The
  HUD does not open account files or select conversation payloads, search indexes or tool output.

Prompts, replies, tool output and attachments are never read for display, stored, or sent
anywhere. Task monitoring does not read account credentials.

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
- Antigravity allowance: the HUD requests Gemini's 5-hour and weekly percentages from the running
  desktop app via `127.0.0.1`. The daemon's local request header is used in memory. HUD handles no
  Google account credentials and stores no Antigravity allowance response.
## Sharing

Do not attach local logs, session files, databases, settings or keys when reporting a bug. A
screenshot or a `render-preview` image is usually enough.
