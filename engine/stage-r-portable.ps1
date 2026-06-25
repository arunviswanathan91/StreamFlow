# StreamFLOW — stage a portable R runtime for bundling into the installer.
#
# Copies the base R install, then installs ONLY the engine's packages into its
# private library (from engine/vendor + CRAN/Bioc). The result is a self-contained
# R-Portable tree the jpackage build bundles, so the shipped app needs no system R.
#
#   powershell -ExecutionPolicy Bypass -File engine\stage-r-portable.ps1
#       [-RHome "C:\Program Files\R\R-4.4.2"] [-Dest "<repo>\app\target\R-Portable"]
#
# Slow (compiles/installs ~1 GB of packages). Re-run only when the R deps change.
param(
    [string]$RHome = "C:\Program Files\R\R-4.4.2",
    [string]$Dest  = ""
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path     # engine\
$repo = Split-Path -Parent $here
if ($Dest -eq "") { $Dest = Join-Path $repo "app\target\R-Portable" }

if (-not (Test-Path (Join-Path $RHome "bin\Rscript.exe"))) { throw "R not found at $RHome" }

Write-Host "Staging R-Portable -> $Dest"
if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

# Copy the base R install (exclude its full library; we install a minimal one).
Write-Host "Copying base R runtime…"
robocopy $RHome $Dest /E /NFL /NDL /NJH /NJS /XD "library" | Out-Null
$lib = Join-Path $Dest "library"
New-Item -ItemType Directory -Force -Path $lib | Out-Null
# Seed with the base/recommended packages that ship with R.
robocopy (Join-Path $RHome "library") $lib /E /NFL /NDL /NJH /NJS | Out-Null

# Install the engine's packages into the portable library only.
Write-Host "Installing engine packages into the portable library (slow)…"
$env:R_LIBS_USER = $lib
$env:R_LIBS = $lib
& (Join-Path $Dest "bin\Rscript.exe") (Join-Path $here "install_from_vendor.R")

Write-Host "Done. Portable R staged at $Dest"
Write-Host "Verify: `"$Dest\bin\Rscript.exe`" -e `"requireNamespace('CytoExploreR')`""
