<#
.SYNOPSIS
  Post-package safety net for StreamFLOW.

.DESCRIPTION
  Inspects electron-builder's unpacked output (dist/win-unpacked by default,
  produced before NSIS compression) and asserts that everything the app needs
  at runtime is actually present. If anything is missing it lists exactly what
  and exits non-zero, so a broken installer can never reach dist/ or be
  published by CI.

  This is the last line of defence against the "R Portable Not Found" class of
  bug regardless of how the build was invoked.

.PARAMETER UnpackedDir
  Path to electron-builder's win-unpacked directory. Defaults to
  <repo>/dist/win-unpacked relative to this script.
#>
[CmdletBinding()]
param(
    [string]$UnpackedDir
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir   = Split-Path -Parent $scriptDir

if (-not $UnpackedDir -or $UnpackedDir.Trim() -eq '') {
    $UnpackedDir = Join-Path $rootDir 'dist\win-unpacked'
}

Write-Host ''
Write-Host '== StreamFLOW package verification ==' -ForegroundColor Cyan
Write-Host "Inspecting: $UnpackedDir"
Write-Host ''

$missing = New-Object System.Collections.Generic.List[string]

function Require-File {
    param([string]$RelPath, [string]$Label)
    $full = Join-Path $UnpackedDir $RelPath
    if (Test-Path -LiteralPath $full -PathType Leaf) {
        Write-Host "  [OK]      $Label" -ForegroundColor Green
    } else {
        Write-Host "  [MISSING] $Label" -ForegroundColor Red
        $script:missing.Add("$Label  ->  $RelPath")
    }
}

function Require-NonEmptyDir {
    param([string]$RelPath, [string]$Label)
    $full = Join-Path $UnpackedDir $RelPath
    if ((Test-Path -LiteralPath $full -PathType Container) -and
        (Get-ChildItem -LiteralPath $full -Force | Select-Object -First 1)) {
        Write-Host "  [OK]      $Label" -ForegroundColor Green
    } else {
        Write-Host "  [MISSING] $Label (directory absent or empty)" -ForegroundColor Red
        $script:missing.Add("$Label  ->  $RelPath")
    }
}

if (-not (Test-Path -LiteralPath $UnpackedDir -PathType Container)) {
    Write-Host "FATAL: unpacked directory not found: $UnpackedDir" -ForegroundColor Red
    Write-Host 'Did electron-builder run? Expected dist\win-unpacked to exist.' -ForegroundColor Red
    exit 1
}

# --- Bundled R runtime (extraResources -> resources/) ---
Require-File        'resources\R-Portable\bin\Rscript.exe'        'R-Portable Rscript.exe'
Require-NonEmptyDir 'resources\R-Portable\library\CytoExploreR'   'R-Portable CytoExploreR library'

# --- Shiny app (extraResources -> resources/) ---
Require-File 'resources\shiny\app.R' 'Shiny app.R'

# --- Packaged Electron app (asar by default, or unpacked app/ dir) ---
$asar       = Join-Path $UnpackedDir 'resources\app.asar'
$appMainDir = Join-Path $UnpackedDir 'resources\app\electron'
if (Test-Path -LiteralPath $asar -PathType Leaf) {
    Write-Host '  [OK]      Electron app (resources\app.asar)' -ForegroundColor Green
    # Best-effort: confirm the three entry files live inside the asar.
    $asarCli = Join-Path $rootDir 'node_modules\.bin\asar.cmd'
    if (Test-Path -LiteralPath $asarCli -PathType Leaf) {
        try {
            $listing = & $asarCli list $asar 2>$null
            foreach ($entry in @('/electron/main.js', '/electron/preload.js', '/electron/splash.html')) {
                if ($listing -contains $entry) {
                    Write-Host "  [OK]      asar:$entry" -ForegroundColor Green
                } else {
                    Write-Host "  [MISSING] asar:$entry" -ForegroundColor Red
                    $missing.Add("asar entry $entry")
                }
            }
        } catch {
            Write-Host '  [WARN]    could not list app.asar contents; skipping inner check' -ForegroundColor Yellow
        }
    }
} elseif (Test-Path -LiteralPath $appMainDir -PathType Container) {
    Require-File 'resources\app\electron\main.js'     'electron/main.js'
    Require-File 'resources\app\electron\preload.js'  'electron/preload.js'
    Require-File 'resources\app\electron\splash.html' 'electron/splash.html'
} else {
    Write-Host '  [MISSING] Electron app (neither resources\app.asar nor resources\app\electron found)' -ForegroundColor Red
    $missing.Add('Electron app payload (app.asar or app/electron)')
}

Write-Host ''
if ($missing.Count -gt 0) {
    Write-Host 'VERIFICATION FAILED. The following are missing from the build:' -ForegroundColor Red
    foreach ($m in $missing) { Write-Host "    - $m" -ForegroundColor Red }
    Write-Host ''
    Write-Host 'This installer would be broken. Aborting before it can be shipped.' -ForegroundColor Red
    exit 1
}

Write-Host 'VERIFICATION PASSED. Installer payload is complete.' -ForegroundColor Green
exit 0
