# Runs cargo inside the workspace toolchain environment, e.g.
#   & .\dev\cargo.ps1 build --release
#   & .\dev\cargo.ps1 test
. (Join-Path $PSScriptRoot 'Enter-NativeDev.ps1') -Quiet
Set-Location (Split-Path -Parent $PSScriptRoot)
& cargo @args
exit $LASTEXITCODE
