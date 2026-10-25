# Install with an agent (Codex or Claude Code)

This is the canonical agent procedure. A repository URL plus "install this" / “帮我安装” is enough.
Do not infer installation commands from other documents.

## Procedure

1. Confirm the repository is `https://github.com/LH-03/agent-monitor-hud` and read
   `install-manifest.json`. Do not read local sessions, prompts, replies, tool output, logs,
   settings values or credentials.
2. Platform: Windows x64 only. Anything else → stop as unsupported.
3. Install for the current user only; never request administrator rights.
4. Download the manifest's `releaseTag` assets — the `windows-x64` asset and `SHA256SUMS.txt` —
   into a new empty folder. Verify the asset's SHA-256 against its line in `SHA256SUMS.txt`.
   A mismatch, a missing line, a network error or a missing asset → stop; never switch to another
   artifact.
5. Extract the ZIP into a new empty folder and run exactly one entrypoint:

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File <extracted>\scripts\install.ps1
   ```

   The script stops without changing anything if it finds an unexpected existing install.
6. Report only the installer's summary lines (version, mode, install path, Claude Code integration
   result). Coming from 3.x, also tell the user where the rollback copy is.
7. After a first install, mention the optional extras and let the user decide: source filters,
   themes, cost estimates, proactive notices, the DeepSeek balance (the user enters their own key
   in Settings → Agents; never enter or handle it for them).

## Codex plugin (optional)

The installer registers the plugin in the personal `local` marketplace. Only when the user asks
Codex to operate the HUD or send notices:

```powershell
codex plugin add codex-monitor-hud@local --json
```

New or updated plugin tools are picked up by a new Codex task.

## Claude Code

When Claude Code is present, the installer runs `AgentMonitorHUD.exe integrate claude`, which adds
only the HUD's lifecycle hooks and the user-scope `agent-monitor-hud` MCP server. Pass `-NoClaude`
to skip it. New Claude Code sessions pick it up.

## Maintenance

| Operation | Command |
| --- | --- |
| Upgrade / repair | run the newer release's `scripts\install.ps1` |
| Roll back to 3.x | `~/plugins/codex-monitor-hud/scripts/rollback.ps1` |
| Uninstall | `~/plugins/codex-monitor-hud/scripts/uninstall.ps1` (`-RemoveSettings` also deletes settings) |

## Language

The HUD follows its saved language; a fresh install starts in English and can be switched to
Simplified Chinese in Settings → General. Never infer language from session content, and never
change an existing saved language during an upgrade.
