# Restores the Codex Monitor HUD 3.x install that Agent Monitor HUD 4.x
# replaced: plugin folder, Codex plugin cache, settings shortcuts, and Claude
# Code configuration (only the HUD's own hooks/MCP entries are removed, so later
# unrelated Claude changes are kept). Settings in %LOCALAPPDATA%\CodexMonitorHUD
# are shared and untouched.
param(
    # The 3.x version to restore; read from the install when omitted.
    [string]$Version
)
$ErrorActionPreference = 'Stop'
$plugins = Join-Path $HOME 'plugins'
$plugin = Join-Path $plugins 'codex-monitor-hud'
$cacheRoot = Join-Path $HOME '.codex\plugins\cache\local\codex-monitor-hud'
# CODEX_MONITOR_HUD_INSTALL_SANDBOX keeps shortcuts under $HOME (install tests).
$sandbox = [bool]$env:CODEX_MONITOR_HUD_INSTALL_SANDBOX
$desktopDir = if ($sandbox) { Join-Path $HOME 'Desktop' } else { [Environment]::GetFolderPath('Desktop') }
$programsDir = if ($sandbox) { Join-Path $HOME 'Programs' } else { [Environment]::GetFolderPath('Programs') }
$desktopLnk = Join-Path $desktopDir 'Codex Monitor HUD Settings.lnk'
$startLnk = Join-Path $programsDir 'Codex Monitor HUD\Open Monitor HUD Settings.lnk'
$newDesktopLnk = Join-Path $desktopDir 'Agent Monitor HUD Settings.lnk'
$newStartFolder = Join-Path $programsDir 'Agent Monitor HUD'

if (-not $Version) {
    $marker = Join-Path $plugin '.rollback-version'
    if (Test-Path $marker) {
        $Version = (Get-Content -Raw $marker).Trim()
    } else {
        $found = Get-ChildItem $plugins -Directory -Filter '.codex-monitor-hud-rollback-3.*' -Force -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($found) { $Version = $found.Name.Substring('.codex-monitor-hud-rollback-'.Length) }
    }
}
if (-not $Version) { throw 'No 3.x rollback copy was found; nothing was changed.' }
$rollback = Join-Path $plugins ".codex-monitor-hud-rollback-$Version"
$backup = Join-Path $rollback '_native-install-backup'
if (-not (Test-Path (Join-Path $rollback '.codex-plugin\plugin.json'))) { throw "No $Version rollback copy at $rollback; nothing was changed." }

# Run from a temporary copy: this script may live inside the folder being replaced.
if ($PSCommandPath -and -not $PSCommandPath.StartsWith($env:TEMP, [StringComparison]::OrdinalIgnoreCase)) {
    $copy = Join-Path $env:TEMP 'agent-monitor-hud-rollback.ps1'
    Copy-Item $PSCommandPath $copy -Force
    & powershell -NoProfile -ExecutionPolicy Bypass -File $copy -Version $Version
    exit $LASTEXITCODE
}

$exe = @((Join-Path $plugin 'bin\AgentMonitorHUD.exe'), (Join-Path $plugin 'bin\CodexMonitorHUD.exe')) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($exe) {
    # Remove only our Claude entries while the binary is still present.
    & $exe unintegrate claude | Out-Null
}
if (-not $sandbox) {
    Get-Process CodexMonitorHUD, AgentMonitorHUD -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$plugin*" } |
        ForEach-Object { $_.CloseMainWindow() | Out-Null; if (-not $_.WaitForExit(1500)) { $_ | Stop-Process -Force } }
}
Start-Sleep -Milliseconds 300

if (Test-Path $plugin) { Remove-Item -Recurse -Force $plugin }
$keepBackup = Join-Path $plugins '.codex-monitor-hud-native-install-backup'
if (Test-Path $keepBackup) { Remove-Item -Recurse -Force $keepBackup }
if (Test-Path $backup) { Move-Item $backup $keepBackup }
Move-Item $rollback $plugin

# Codex's cache held a copy of the 3.x plugin folder; drop the 4.x copies.
if (Test-Path $cacheRoot) {
    robocopy $plugin (Join-Path $cacheRoot $Version) /MIR /NFL /NDL /NJH /NJS /NP /R:1 /W:1 | Out-Null
    Get-ChildItem $cacheRoot -Directory | Where-Object { $_.Name -like '4.*' } | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
}

Remove-Item $newDesktopLnk -ErrorAction SilentlyContinue
if (Test-Path $newStartFolder) { Remove-Item -Recurse -Force $newStartFolder }
$shortcuts = Join-Path $keepBackup 'shortcuts'
if (Test-Path (Join-Path $shortcuts 'desktop.lnk')) { Copy-Item (Join-Path $shortcuts 'desktop.lnk') $desktopLnk -Force }
if (Test-Path (Join-Path $shortcuts 'startmenu.lnk')) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $startLnk) | Out-Null
    Copy-Item (Join-Path $shortcuts 'startmenu.lnk') $startLnk -Force
}

Write-Output "Restored $Version at $plugin. Restart Codex to reload the plugin."
if (Test-Path $keepBackup) { Write-Output "Backups of the 4.x install remain in $keepBackup (Claude config copies under claude\)." }
