# StreamFLOW - smoke test of a built jpackage app-image, run BEFORE it is wrapped into an installer.
#
# Every ship-blocker this project has had was the same shape: the build went green and produced a
# package whose app could not start or whose engine was missing (mvn clean wiping the staged
# Python, appDir pointing at a directory that never exists, a main class the JVM refuses to launch,
# a staged Python with no packages). None of those are visible to `mvn test`. This script checks
# the artifact itself:
#
#   1. the image has the launcher, the engine script and the bundled Python
#   2. the BUNDLED Python runs the BUNDLED engine and answers a `capabilities` request with every
#      required package present
#   3. StreamFLOW.exe starts, stays up, and spawns its engine from inside the image
#
#   powershell -File app\smoke-test.ps1 -Image app\target\image\StreamFLOW
#       [-LaunchSeconds 60]   how long to wait for the app to bring its engine up
#       [-SkipLaunch]         checks 1 and 2 only (no desktop session available)
param(
    [Parameter(Mandatory = $true)][string]$Image,
    [int]$LaunchSeconds = 60,
    [switch]$SkipLaunch
)

$ErrorActionPreference = "Stop"
$Image = (Resolve-Path $Image).Path

function Fail([string]$msg) { throw "SMOKE TEST FAILED: $msg" }

# --- 1. layout ---------------------------------------------------------------------------------
$exe    = Join-Path $Image "StreamFLOW.exe"
$engine = Join-Path $Image "content\engine\streamflow_engine.py"
$python = Join-Path $Image "content\python\python.exe"
foreach ($p in @($exe, $engine, $python)) {
    if (-not (Test-Path $p)) { Fail "missing from the image: $p" }
}
Write-Host "[1/3] layout ok: launcher, engine script and bundled Python are present"

# --- 2. bundled Python runs the bundled engine -------------------------------------------------
# PYTHONHOME/PYTHONPATH from the build machine must not leak in: the point is to prove the image
# is self-contained.
$env:PYTHONHOME = $null
$env:PYTHONPATH = $null
$req = Join-Path $env:TEMP ("sf-smoke-" + $PID + ".jsonl")
Set-Content -Path $req -Encoding ASCII -Value @('{"id":1,"cmd":"capabilities"}', '__shutdown__')
try {
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"      # a warning on the engine's stderr is not a failure
    $out = Get-Content $req | & $python $engine 2>$null
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
} finally {
    Remove-Item $req -Force -ErrorAction SilentlyContinue
}
if ($code -ne 0) { Fail "the bundled engine exited with code $code" }

$ready = $null; $caps = $null
foreach ($line in @($out)) {
    if (-not $line -or -not $line.TrimStart().StartsWith("{")) { continue }
    try { $msg = $line | ConvertFrom-Json } catch { continue }
    if ($msg.type -eq "ready")  { $ready = $msg }
    if ($msg.type -eq "result" -and $msg.id -eq 1) { $caps = $msg }
    if ($msg.type -eq "error")  { Fail "the engine answered with an error: $($msg.message)" }
}
if (-not $ready) { Fail "the engine never sent its 'ready' line. Output was:`n$($out -join "`n")" }
if (-not $caps)  { Fail "the engine did not answer 'capabilities'. Output was:`n$($out -join "`n")" }

$required = @("flowkit", "flowio", "flowutils", "numpy", "scipy", "pandas", "matplotlib",
              "sklearn", "umap", "openTSNE", "phenograph", "flowsom")
$missing = @($required | Where-Object { -not $caps.packages.$_ })
if ($missing.Count -gt 0) { Fail "bundled Python is missing: $($missing -join ', ')" }
Write-Host "[2/3] engine ok: bundled Python $($ready.py) answered, all $($required.Count) required packages present"

# --- 3. the launcher starts and brings its own engine up ---------------------------------------
if ($SkipLaunch) {
    Write-Host "[3/3] launch check skipped (-SkipLaunch)"
    Write-Host "SMOKE TEST PASSED (launch not checked)"
    exit 0
}

$pyPrefix = (Join-Path $Image "content\python").ToLower()
$app = Start-Process -FilePath $exe -PassThru
$engineProc = $null
try {
    $deadline = (Get-Date).AddSeconds($LaunchSeconds)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 2
        if ($app.HasExited) {
            Fail ("StreamFLOW.exe exited on its own after launch (exit code $($app.ExitCode)). " +
                  "It is a windowed binary, so a startup error is invisible to a user: the app just does nothing.")
        }
        $engineProc = Get-CimInstance Win32_Process -Filter "Name = 'python.exe'" |
            Where-Object { $_.ExecutablePath -and $_.ExecutablePath.ToLower().StartsWith($pyPrefix) } |
            Select-Object -First 1
        if ($engineProc) { break }
    }
    if (-not $engineProc) {
        Fail "StreamFLOW.exe stayed up but started no engine from $pyPrefix within $LaunchSeconds s"
    }
    # Still alive a moment after the engine came up: rules out a crash right behind the splash.
    Start-Sleep -Seconds 5
    if ($app.HasExited) { Fail "StreamFLOW.exe exited right after starting its engine (exit code $($app.ExitCode))" }
    Write-Host "[3/3] launch ok: StreamFLOW.exe is running and started its engine (python pid $($engineProc.ProcessId))"
} finally {
    # Kill the app and everything it spawned. taskkill /T walks the process tree.
    if (-not $app.HasExited) { & taskkill.exe /PID $app.Id /T /F 2>$null | Out-Null }
    Get-CimInstance Win32_Process -Filter "Name = 'python.exe'" |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.ToLower().StartsWith($pyPrefix) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

Write-Host "SMOKE TEST PASSED"
