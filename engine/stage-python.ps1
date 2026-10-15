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
# SHA-256 of $asset, taken from the publisher's .sha256 sidecar. Pinned so an altered or truncated
# download fails loudly instead of silently producing a different engine on a different machine.
# When bumping $ver/$tag, update this too (curl "$url.sha256").
$sha256  = "4c71d25731214b8a960d1d87510f24179d819249c5b434aaf7135818421b6215"

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

Write-Host "Verifying SHA-256..."
$actual = (Get-FileHash -Algorithm SHA256 -Path $tmp).Hash.ToLower()
if ($actual -ne $sha256.ToLower()) {
    Remove-Item $tmp -Force -ErrorAction SilentlyContinue
    throw "Checksum mismatch for $asset`n  expected: $sha256`n  actual:   $actual`nRefusing to stage an unverified runtime."
}
Write-Host "  OK ($actual)"

Write-Host "Extracting..."
# The install_only archive extracts a top-level 'python' folder. Extract into the
# parent of $Dest so the folder lands exactly at $Dest (default name 'python').
# Use the Windows bsdtar explicitly. A bare `tar` can resolve to Git Bash's GNU tar when its
# usr\bin is on PATH, and that one reads "C:\\..." as a remote host spec:
#   tar (child): Cannot connect to C: resolve failed
$tarExe = Join-Path $env:SystemRoot "System32\tar.exe"
if (-not (Test-Path $tarExe)) { $tarExe = "tar" }   # non-Windows / older builds
& $tarExe -xzf $tmp -C $parent
if ($LASTEXITCODE -ne 0) { throw "tar extraction failed (exit $LASTEXITCODE) using $tarExe" }
Remove-Item $tmp -Force

$pyExe = Join-Path $Dest "python.exe"
if (-not (Test-Path $pyExe)) {
    throw "python.exe not found at $pyExe after extraction - check the archive layout or pinned tag."
}

# --- install the engine's Python packages -----------------------------------
# TLS-intercepting AV (Avast HTTPS scanning) re-signs pypi.org with a local root that the bundled
# CPython's certifi does not carry, so pip dies with CERTIFICATE_VERIFY_FAILED. The repo ships an
# exported Windows root store for exactly this (engine\export-win-ca.ps1 regenerates it).
$caBundle = Join-Path $here "win-ca-bundle.pem"
if (Test-Path $caBundle) {
    Write-Host "Using Windows CA bundle for pip: $caBundle"
    $env:PIP_CERT = $caBundle
    $env:SSL_CERT_FILE = $caBundle
} else {
    Write-Warning "engine\win-ca-bundle.pem not found - pip may fail behind a TLS-intercepting proxy/AV. Regenerate with engine\export-win-ca.ps1"
}

Write-Host "Upgrading pip and installing engine packages..."
& $pyExe -m pip install --upgrade pip
if ($LASTEXITCODE -ne 0) { throw "pip self-upgrade failed (exit $LASTEXITCODE)." }
# flowkit pulls flowio, flowutils, numpy, scipy, pandas; matplotlib drives the
# render/stats commands; scikit-learn/umap-learn/openTSNE power Dim. Reduction,
# Clustering and the Classifier. FlowCal provides bead-based MEF/MEFL calibration.
# Keep this in sync with the CI install in build-java.yml.
#
# DO NOT add `cytonormpy`: it hard-pins flowio<=1.3.0 while flowkit 1.3.2 requires
# flowio>=1.4.0 — mutually exclusive, and flowkit is the core engine. CytoNorm is an R
# package (engine/vendor/CytoNorm-master) and runs through the optional R runway instead.
& $pyExe -m pip install flowkit matplotlib scikit-learn umap-learn openTSNE phenograph flowsom FlowCal
if ($LASTEXITCODE -ne 0) { throw "pip install of the engine packages failed (exit $LASTEXITCODE)." }

# Verification must be FATAL. Before this guard the script printed "Done" and exited 0 after every
# install had failed, staging a package-less Python that jpackage would happily bundle - producing
# an installer whose engine cannot start. Same failure class as the `mvn clean` ship-blocker.
Write-Host "Verifying..."
$required = @("flowkit", "FlowCal", "matplotlib", "sklearn", "umap", "openTSNE")
$missing = @()
foreach ($mod in $required) {
    & $pyExe -c "import $mod" 2>$null
    if ($LASTEXITCODE -ne 0) { $missing += $mod }
}
if ($missing.Count -gt 0) {
    throw "Staged Python is incomplete - these imports failed: $($missing -join ', '). Refusing to report success; the packaged app would ship a dead engine."
}
& $pyExe -c "import flowkit; print('SUCCESS: staged FlowKit', flowkit.__version__)"
& $pyExe -m pip check

Write-Host "Done. Portable Python staged at $Dest"
Write-Host "Next: powershell -File app\package.ps1   (bundles Python + R-Portable into the EXE)"
