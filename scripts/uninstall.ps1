# Removes Agent Monitor HUD 4.x for the current user: the HUD's Claude Code
# hooks/MCP entries, the plugin folder, its Codex cache copies, the personal
# marketplace entry and the settings shortcuts.
#
# Kept: settings in %LOCALAPPDATA%\CodexMonitorHUD (pass -RemoveSettings to
# delete them too) and any 3.x rollback copy under ~/plugins.
param([switch]$RemoveSettings)
$ErrorActionPreference = 'Stop'
$pluginName = 'codex-monitor-hud'
$plugins = Join-Path $HOME 'plugins'
$plugin = Join-Path $plugins $pluginName
$cacheRoot = Join-Path $HOME ".codex\plugins\cache\local\$pluginName"
$marketplacePath = Join-Path $HOME '.agents\plugins\marketplace.json'
$sandbox = [bool]$env:CODEX_MONITOR_HUD_INSTALL_SANDBOX
$desktopDir = if ($sandbox) { Join-Path $HOME 'Desktop' } else { [Environment]::GetFolderPath('Desktop') }
$programsDir = if ($sandbox) { Join-Path $HOME 'Programs' } else { [Environment]::GetFolderPath('Programs') }

# Run from a temporary copy: this script may live inside the folder being removed.
if ($PSCommandPath -and -not $PSCommandPath.StartsWith($env:TEMP, [StringComparison]::OrdinalIgnoreCase)) {
    $copy = Join-Path $env:TEMP 'agent-monitor-hud-uninstall.ps1'
    Copy-Item $PSCommandPath $copy -Force
    & powershell -NoProfile -ExecutionPolicy Bypass -File $copy @(if ($RemoveSettings) { '-RemoveSettings' })
    exit $LASTEXITCODE
}

$exe = Join-Path $plugin 'bin\AgentMonitorHUD.exe'
if (Test-Path $exe) { & $exe unintegrate claude | Out-Null }
if (-not $sandbox) {
    Get-Process AgentMonitorHUD -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$plugin*" } |
        ForEach-Object { $_.CloseMainWindow() | Out-Null; if (-not $_.WaitForExit(1500)) { $_ | Stop-Process -Force } }
}
Start-Sleep -Milliseconds 300

if (Test-Path $plugin) { Remove-Item -Recurse -Force $plugin }
if (Test-Path $cacheRoot) { Remove-Item -Recurse -Force $cacheRoot -ErrorAction SilentlyContinue }
if (Test-Path $marketplacePath) {
    $marketplace = Get-Content -Raw -Encoding UTF8 $marketplacePath | ConvertFrom-Json
    $marketplace.plugins = @($marketplace.plugins | Where-Object { $_.name -ne $pluginName })
    [IO.File]::WriteAllText($marketplacePath, ($marketplace | ConvertTo-Json -Depth 8), (New-Object Text.UTF8Encoding $false))
}
Remove-Item (Join-Path $desktopDir 'Agent Monitor HUD Settings.lnk') -ErrorAction SilentlyContinue
$startFolder = Join-Path $programsDir 'Agent Monitor HUD'
if (Test-Path $startFolder) { Remove-Item -Recurse -Force $startFolder }

$state = Join-Path $env:LOCALAPPDATA 'CodexMonitorHUD'
if ($RemoveSettings -and (Test-Path $state)) {
    Remove-Item -Recurse -Force $state
    Write-Output "Removed settings at $state"
} else {
    Write-Output "Settings kept at $state"
}
Write-Output 'Agent Monitor HUD was uninstalled.'
