# StreamFLOW - stage a portable Python runtime for bundling into the installer.
#
# Downloads a relocatable standalone CPython (python-build-standalone, the same
# distribution uv/Rye use) and installs the engine's Python packages into it. The
# result is a self-contained Python tree (no system Python, no venv-to-base links)
# that the jpackage build bundles next to R-Portable, so the shipped app needs no
# Python, R, or Java installed ("fat binary").
#
#   powershell -ExecutionPolicy Bypass -File engine\stage-python.ps1
#       [-Dest "<repo>\app\target\python"]
#
# Slow on first run (downloads ~30 MB CPython + installs FlowKit/numpy/scipy).
# Re-run only when the Python deps change.
param([string]$Dest = "")

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path     # engine\
$repo = Split-Path -Parent $here
if ($Dest -eq "") { $Dest = Join-Path $repo "app\target\python" }

# --- standalone CPython release ---------------------------------------------
# Pinned to a python-build-standalone 'install_only' build (flattened tree with
# python.exe + Lib + Scripts at the root). NOTE: newer releases live under the
# astral-sh/python-build-standalone repo - bump $tag/$ver and $repoOrg together.
$ver     = "3.11.9"
$tag     = "20240814"
$repoOrg = "indygreg"
$asset   = "cpython-$ver+$tag-x86_64-pc-windows-msvc-install_only.tar.gz"
$url     = "https://github.com/$repoOrg/python-build-standalone/releases/download/$tag/$asset"

Write-Host "Staging portable Python -> $Dest"
if (Test-Path $Dest) {
    Write-Host "Cleaning existing directory..."
    Remove-Item -Recurse -Force $Dest
}
$parent = Split-Path $Dest
New-Item -ItemType Directory -Force -Path $parent | Out-Null

# --- download + extract ------------------------------------------------------
$tmp = Join-Path $env:TEMP "sf-py-standalone.tar.gz"
Write-Host "Downloading CPython $ver ($tag)..."
Invoke-WebRequest $url -OutFile $tmp

Write-Host "Extracting..."
# The install_only archive extracts a top-level 'python' folder. Extract into the
# parent of $Dest so the folder lands exactly at $Dest (default name 'python').
tar -xzf $tmp -C $parent
Remove-Item $tmp -Force

$pyExe = Join-Path $Dest "python.exe"
if (-not (Test-Path $pyExe)) {
    throw "python.exe not found at $pyExe after extraction - check the archive layout or pinned tag."
}

# --- install the engine's Python packages -----------------------------------
Write-Host "Upgrading pip and installing engine packages..."
& $pyExe -m pip install --upgrade pip
# flowkit pulls flowio, flowutils, numpy, scipy, pandas; matplotlib drives the
# render/stats commands; scikit-learn/umap-learn/openTSNE power Dim. Reduction,
# Clustering and the Classifier. Keep this in sync with the CI install in build-java.yml.
& $pyExe -m pip install flowkit matplotlib scikit-learn umap-learn openTSNE phenograph flowsom

Write-Host "Verifying..."
& $pyExe -c "import flowkit; print('SUCCESS: staged FlowKit', flowkit.__version__)"

Write-Host "Done. Portable Python staged at $Dest"
Write-Host "Next: powershell -File app\package.ps1   (bundles Python + R-Portable into the EXE)"
