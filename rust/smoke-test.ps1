# StreamFLOW (native) - start the built binary and prove it draws.
#
# The release binary is a windowed program: if it dies on startup the user sees nothing at all.
# This starts it with its own --screenshot flag, which lets the app run its event loop, save one
# frame as a PNG and close. A non-empty PNG on disk means the window opened and frames were drawn.
#
#   powershell -File rust\smoke-test.ps1 -Exe <path>\streamflow.exe [-TimeoutSeconds 90]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
$shot = Join-Path $env:TEMP ("sf-native-smoke-" + $PID + ".png")
Remove-Item $shot -Force -ErrorAction SilentlyContinue

$p = Start-Process -FilePath $Exe -ArgumentList "--screenshot=$shot" -PassThru
if (-not $p.WaitForExit($TimeoutSeconds * 1000)) {
    & taskkill.exe /PID $p.Id /T /F 2>$null | Out-Null
    throw "SMOKE TEST FAILED: streamflow.exe did not finish within $TimeoutSeconds s (no frame was captured)."
}
if ($p.ExitCode -ne 0) {
    throw "SMOKE TEST FAILED: streamflow.exe exited with code $($p.ExitCode) before drawing a frame."
}
if (-not (Test-Path $shot)) {
    throw "SMOKE TEST FAILED: streamflow.exe exited cleanly but wrote no screenshot."
}
$bytes = (Get-Item $shot).Length
Remove-Item $shot -Force -ErrorAction SilentlyContinue
if ($bytes -lt 2000) {
    throw "SMOKE TEST FAILED: the screenshot is only $bytes bytes - nothing was drawn."
}
Write-Host "SMOKE TEST PASSED: streamflow.exe started, drew frames and saved a $bytes-byte screenshot."
