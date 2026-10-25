# Installs or upgrades Agent Monitor HUD 4.x (formerly Codex Monitor HUD) for
# the current user. No administrator rights are needed.
#
# Works from an extracted release ZIP (bin\AgentMonitorHUD.exe next to the
# plugin files) or from a source checkout after `cargo build --release`.
#
#   fresh install   ~/plugins/codex-monitor-hud is created
#   from 3.x        the old plugin folder is kept whole as
#                   ~/plugins/.codex-monitor-hud-rollback-<version>, together with
#                   _native-install-backup (old shortcuts, Claude config copies);
#                   scripts\rollback.ps1 restores it
#   from 4.x        upgraded in place
#
# Settings in %LOCALAPPDATA%\CodexMonitorHUD are shared by every version and
# never touched here.
param(
    # Path to AgentMonitorHUD.exe; found automatically when omitted.
    [string]$Exe,
    # Do not start the HUD after installing.
    [switch]$NoStart,
    # Skip the Claude Code hooks / MCP registration even if Claude Code is present.
    [switch]$NoClaude
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$pluginName = 'codex-monitor-hud'

# Release ZIP: the plugin files sit next to scripts\. Source checkout: package\.
$releaseLayout = Test-Path (Join-Path $root '.codex-plugin\plugin.json')
$packageDir = if ($releaseLayout) { $root } else { Join-Path $root 'package' }
if (-not $Exe) {
    $candidates = @(
        (Join-Path $root 'bin\AgentMonitorHUD.exe'),
        (Join-Path $root 'target\release\AgentMonitorHUD.exe'),
        (Join-Path $root '..\.native-build\release\AgentMonitorHUD.exe')
    )
    $Exe = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Exe) { throw 'AgentMonitorHUD.exe not found. Run `cargo build --release` first, or pass -Exe.' }
}
$Exe = (Resolve-Path $Exe).Path
$newVersion = [string](Get-Content -Raw -Encoding UTF8 (Join-Path $packageDir '.codex-plugin\plugin.json') | ConvertFrom-Json).version

$plugins = Join-Path $HOME 'plugins'
$plugin = Join-Path $plugins $pluginName
$cacheRoot = Join-Path $HOME ".codex\plugins\cache\local\$pluginName"
$marketplacePath = Join-Path $HOME '.agents\plugins\marketplace.json'
# CODEX_MONITOR_HUD_INSTALL_SANDBOX keeps shortcuts under $HOME (install tests).
$sandbox = [bool]$env:CODEX_MONITOR_HUD_INSTALL_SANDBOX
$desktopDir = if ($sandbox) { Join-Path $HOME 'Desktop' } else { [Environment]::GetFolderPath('Desktop') }
$programsDir = if ($sandbox) { Join-Path $HOME 'Programs' } else { [Environment]::GetFolderPath('Programs') }
# 3.x shortcut names (backed up on migration, replaced by the new names).
$desktopLnk = Join-Path $desktopDir 'Codex Monitor HUD Settings.lnk'
$startFolder = Join-Path $programsDir 'Codex Monitor HUD'
$startLnk = Join-Path $startFolder 'Open Monitor HUD Settings.lnk'
$newDesktopLnk = Join-Path $desktopDir 'Agent Monitor HUD Settings.lnk'
$newStartFolder = Join-Path $programsDir 'Agent Monitor HUD'
$newStartLnk = Join-Path $newStartFolder 'Agent Monitor HUD Settings.lnk'
$utf8 = New-Object Text.UTF8Encoding $false

function Get-PluginVersion([string]$dir) {
    $manifest = Join-Path $dir '.codex-plugin\plugin.json'
    if (-not (Test-Path $manifest)) { return $null }
    return [string](Get-Content -Raw -Encoding UTF8 $manifest | ConvertFrom-Json).version
}

function Stop-Hud {
    if ($sandbox) { return }
    # Stop installed HUD modes before replacing their executable. Development
    # binaries and unrelated processes with the same filename are left alone.
    Get-Process CodexMonitorHUD, AgentMonitorHUD -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and ($_.Path.StartsWith($plugin + '\', [StringComparison]::OrdinalIgnoreCase) -or $_.Path.StartsWith($cacheRoot + '\', [StringComparison]::OrdinalIgnoreCase))
    } |
        ForEach-Object { $_.CloseMainWindow() | Out-Null; if (-not $_.WaitForExit(1500)) { $_ | Stop-Process -Force } }
    Get-CimInstance Win32_Process -Filter "Name='dotnet.exe' OR Name='CodexMonitorHud.exe'" |
        Where-Object { $_.CommandLine -match 'CodexMonitorHud\.dll|codex-monitor-hud\\runtime' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

function Write-Shortcut([string]$path, [string]$target) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($path)
    $link.TargetPath = $target
    $link.Arguments = '--open-settings'
    $link.WorkingDirectory = Split-Path -Parent $target
    $link.IconLocation = "$target,0"
    $link.Description = 'Agent Monitor HUD settings'
    $link.Save()
}

function Update-Marketplace {
    # Same personal-marketplace entry 3.x used, so `codex plugin add
    # codex-monitor-hud@local` keeps working. Unrelated entries are kept.
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $marketplacePath) | Out-Null
    if (Test-Path $marketplacePath) {
        $marketplace = Get-Content -Raw -Encoding UTF8 $marketplacePath | ConvertFrom-Json
    } else {
        $marketplace = [pscustomobject]@{ name = 'personal'; interface = [pscustomobject]@{ displayName = 'Personal' }; plugins = @() }
    }
    $entry = [pscustomobject]@{
        name = $pluginName
        source = [pscustomobject]@{ source = 'local'; path = "./plugins/$pluginName" }
        policy = [pscustomobject]@{ installation = 'AVAILABLE'; authentication = 'ON_INSTALL' }
        category = 'Productivity'
    }
    $marketplace.plugins = @(@($marketplace.plugins | Where-Object { $_.name -ne $pluginName }) + $entry)
    $temporary = "$marketplacePath.tmp"
    [IO.File]::WriteAllText($temporary, ($marketplace | ConvertTo-Json -Depth 8), $utf8)
    Move-Item $temporary $marketplacePath -Force
}

$current = Get-PluginVersion $plugin
$migration = $current -and $current.StartsWith('3.')
if ($current -and -not $migration -and -not $current.StartsWith('4.')) {
    throw "Unexpected installed plugin version '$current' at $plugin; nothing was changed."
}
if ((Test-Path $plugin) -and -not $current) {
    throw "$plugin exists but has no plugin manifest; move it aside first. Nothing was changed."
}
$rollback = if ($migration) { Join-Path $plugins ".codex-monitor-hud-rollback-$current" } else { $null }
if ($migration -and (Test-Path $rollback)) {
    throw "A rollback copy already exists at $rollback while $current is installed; resolve it manually first. Nothing was changed."
}

$mode = if ($migration) { "migration from $current" } elseif ($current) { "upgrade from $current" } else { 'fresh install' }
Write-Output "Agent Monitor HUD $newVersion ($mode)"
Stop-Hud

if ($migration) {
    # Move first: the backup folder lives inside the rollback copy.
    Move-Item $plugin $rollback
    $backup = Join-Path $rollback '_native-install-backup'
    $shortcuts = Join-Path $backup 'shortcuts'
    $claude = Join-Path $backup 'claude'
    New-Item -ItemType Directory -Force -Path $shortcuts, $claude | Out-Null
    if (Test-Path $desktopLnk) { Copy-Item $desktopLnk (Join-Path $shortcuts 'desktop.lnk') }
    if (Test-Path $startLnk) { Copy-Item $startLnk (Join-Path $shortcuts 'startmenu.lnk') }
    foreach ($f in @((Join-Path $HOME '.claude\settings.json'), (Join-Path $HOME '.claude.json'))) {
        if (Test-Path $f) { Copy-Item $f (Join-Path $claude (Split-Path -Leaf $f)) }
    }
    Copy-Item (Join-Path $PSScriptRoot 'rollback.ps1') (Join-Path $backup "rollback-to-$current.ps1")
    Write-Output "Previous $current kept at $rollback"
}

# Lay out the plugin in a staging folder, then switch.
$stage = Join-Path $plugins '.codex-monitor-hud-staging'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
robocopy $packageDir $stage /E /XD bin scripts .git target /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -gt 7) { throw 'Could not stage the plugin files; installation stopped.' }
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'bin'), (Join-Path $stage 'scripts') | Out-Null
Copy-Item $Exe (Join-Path $stage 'bin\AgentMonitorHUD.exe')
foreach ($script in 'install.ps1', 'rollback.ps1', 'uninstall.ps1') {
    Copy-Item (Join-Path $PSScriptRoot $script) (Join-Path $stage "scripts\$script")
}
$exeInstalled = Join-Path $plugin 'bin\AgentMonitorHUD.exe'
$mcp = [ordered]@{ mcpServers = [ordered]@{ 'agent-monitor-hud' = [ordered]@{ command = $exeInstalled; args = @('mcp') } } }
[IO.File]::WriteAllText((Join-Path $stage '.mcp.json'), ($mcp | ConvertTo-Json -Depth 5), $utf8)
if ($migration) { [IO.File]::WriteAllText((Join-Path $stage '.rollback-version'), $current, $utf8) }
elseif (Test-Path (Join-Path $plugin '.rollback-version')) { Copy-Item (Join-Path $plugin '.rollback-version') $stage }
if (Test-Path $plugin) { Remove-Item -Recurse -Force $plugin }
Move-Item $stage $plugin

# Codex runs plugins from its cache: refresh every cached version folder (old
# Node MCP servers keep running from memory until their Codex session ends; the
# native HUD keeps the legacy heartbeat fresh so they never relaunch 3.x).
if ((Test-Path $cacheRoot) -or (Test-Path (Join-Path $HOME '.codex'))) {
    New-Item -ItemType Directory -Force -Path (Join-Path $cacheRoot $newVersion) | Out-Null
    foreach ($dir in Get-ChildItem $cacheRoot -Directory) {
        robocopy $plugin $dir.FullName /MIR /NFL /NDL /NJH /NJS /NP /R:1 /W:1 | Out-Null
        if ($LASTEXITCODE -gt 7) { throw 'Could not update the Codex plugin cache.' }
    }
}
Update-Marketplace

# Settings shortcuts open the native exe (3.x originals are in the backup).
foreach ($old in @($desktopLnk, $startLnk)) { Remove-Item $old -ErrorAction SilentlyContinue }
if ((Test-Path $startFolder) -and -not (Get-ChildItem $startFolder -Force)) { Remove-Item $startFolder }
Write-Shortcut $newDesktopLnk $exeInstalled
Write-Shortcut $newStartLnk $exeInstalled

# Claude Code: lifecycle hooks + MCP server (only the HUD's own entries), and
# only when Claude Code is present on this machine.
$hasClaude = (Test-Path (Join-Path $HOME '.claude')) -or (Test-Path (Join-Path $HOME '.claude.json'))
if ($hasClaude -and -not $NoClaude) {
    & $exeInstalled integrate claude | Out-Null
    Write-Output "Claude Code integration: $(if ($LASTEXITCODE -eq 0) { 'ok' } else { 'FAILED (see %LOCALAPPDATA%\CodexMonitorHUD\hud.log)' })"
} else {
    Write-Output 'Claude Code integration: skipped'
}

if (-not $NoStart) {
    Remove-Item (Join-Path $env:LOCALAPPDATA 'CodexMonitorHUD\manual-exit.signal') -ErrorAction SilentlyContinue
    Start-Process $exeInstalled
}
Write-Output "Installed Agent Monitor HUD $newVersion at $plugin"
