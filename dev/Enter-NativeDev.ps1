[CmdletBinding()]
param([switch]$Quiet)

# Dot-source this file in the terminal that will run Cargo: it loads the MSVC
# x64 build environment into this process only. No global settings change.
#
# Optional private toolchain: when a `.tooling` folder (cargo + rustup) exists
# two levels above this checkout, it is used and the build cache goes to a
# sibling `.native-build` folder. Otherwise the normal rustup install and
# `target\` are used.
$nativeHudRoot = Split-Path -Parent $PSScriptRoot
$nativeHudWorkspace = Split-Path -Parent (Split-Path -Parent $nativeHudRoot)
$nativeHudTooling = Join-Path $nativeHudWorkspace '.tooling'
if (Test-Path -LiteralPath (Join-Path $nativeHudTooling 'cargo\bin\rustc.exe')) {
    $env:CARGO_HOME = Join-Path $nativeHudTooling 'cargo'
    $env:RUSTUP_HOME = Join-Path $nativeHudTooling 'rustup'
    $env:CARGO_TARGET_DIR = Join-Path (Split-Path -Parent $nativeHudRoot) '.native-build'
}
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '1'
$env:CARGO_PROFILE_TEST_DEBUG = '1'

$nativeHudVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$nativeHudVsRoot = & $nativeHudVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($nativeHudVsRoot)) {
    throw 'MSVC x64 Build Tools were not found (Visual Studio Build Tools with the C++ workload are required).'
}
$nativeHudDevCmd = Join-Path $nativeHudVsRoot 'Common7\Tools\VsDevCmd.bat'
$nativeHudCommand = 'call "{0}" -no_logo -arch=x64 -host_arch=x64 >nul && set' -f $nativeHudDevCmd
$nativeHudEnvironment = & "$env:SystemRoot\System32\cmd.exe" /d /c $nativeHudCommand
if ($LASTEXITCODE -ne 0) { throw 'MSVC environment activation failed.' }
$nativeHudEnvironmentKeys = @(
    'PATH', 'INCLUDE', 'LIB', 'LIBPATH', 'VCINSTALLDIR', 'VCToolsInstallDir',
    'VCToolsVersion', 'WindowsSdkDir', 'WindowsSDKVersion', 'WindowsSDKLibVersion',
    'UniversalCRTSdkDir', 'UCRTVersion', 'VSINSTALLDIR', 'VSCMD_VER',
    'VSCMD_ARG_HOST_ARCH', 'VSCMD_ARG_TGT_ARCH'
)
foreach ($nativeHudEntry in $nativeHudEnvironment) {
    $nativeHudPair = $nativeHudEntry -split '=', 2
    if ($nativeHudPair.Count -eq 2 -and $nativeHudEnvironmentKeys -contains $nativeHudPair[0]) {
        [Environment]::SetEnvironmentVariable($nativeHudPair[0], $nativeHudPair[1], 'Process')
    }
}
if ($env:CARGO_HOME) {
    $nativeHudCargoBin = Join-Path $env:CARGO_HOME 'bin'
    if (($env:PATH -split ';') -notcontains $nativeHudCargoBin) {
        $env:PATH = $nativeHudCargoBin + ';' + $env:PATH
    }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo was not found. Install Rust from https://rustup.rs (the pinned version is in rust-toolchain.toml).'
}
if (-not $Quiet) {
    Write-Host ('Development environment ready. Build cache: ' + $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }))
}
