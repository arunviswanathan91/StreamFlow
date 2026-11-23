# StreamFLOW (native) - static checks on the release binary. No window is opened.
#
# The native build ships as ONE file that a user downloads and double-clicks. Each check below is a
# way that goes wrong without any test failing:
#
#   * it imports VCRUNTIME140.dll / MSVCP140.dll  -> "VCRUNTIME140.dll was not found" on a clean
#     Windows install (fixed by +crt-static in rust/.cargo/config.toml; this guards the fix)
#   * it is a console program                     -> a black console window opens behind the app
#   * it has no icon resource                     -> the generic Windows program icon everywhere
#
#   powershell -File rust\check-binary.ps1 -Exe <path>\streamflow.exe
param([Parameter(Mandatory = $true)][string]$Exe)

$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
$bytes = [System.IO.File]::ReadAllBytes($Exe)
$problems = @()

# --- PE header: subsystem ------------------------------------------------------------------------
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x00004550) { throw "$Exe is not a PE executable." }
$subsystem = [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
if ($subsystem -ne 2) {
    $problems += "subsystem is $subsystem, expected 2 (windowed). A console window would open with the app."
}

# --- imported DLL names --------------------------------------------------------------------------
# Latin-1 keeps one char per byte, so a plain text search over the whole image finds import names.
$text = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
foreach ($dll in @("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll")) {
    if ($text.IndexOf($dll, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        $problems += "imports $dll - the C runtime is not linked statically, so the exe fails on a PC without the Visual C++ redistributable."
    }
}

# --- icon and version resources ------------------------------------------------------------------
$info = (Get-Item $Exe).VersionInfo
if ($info.ProductName -ne "StreamFLOW") {
    $problems += "no version resource (ProductName is '$($info.ProductName)') - build.rs did not embed resources, so there is no icon either. Is the Windows resource compiler (rc.exe) available?"
}

if ($problems.Count -gt 0) {
    $problems | ForEach-Object { Write-Host "  PROBLEM: $_" }
    throw "BINARY CHECK FAILED: $($problems.Count) problem(s) with $Exe"
}
$mb = [math]::Round($bytes.Length / 1MB, 1)
Write-Host "BINARY CHECK PASSED: $mb MB, windowed, C runtime static, icon + version resources embedded ($($info.ProductName) $($info.FileVersion))."
