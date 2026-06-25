# StreamFLOW (Java) — dev launcher
# Pins JAVA_HOME to the bundled Microsoft JDK 25 (LTS) and runs the JavaFX app
# via the javafx-maven-plugin.
#
# Usage:
#   ./run-dev.ps1                       # uses Rscript from PATH / bundled R-Portable
#   ./run-dev.ps1 -Rscript "C:\R\bin\Rscript.exe"
#
param(
    [string]$Rscript = ""
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $here

# --- locate R and export R_HOME ---------------------------------------------
# The app finds R via R_HOME (inherited by the forked app JVM and the Rscript
# child). Default to a standard install if not supplied.
if ($Rscript -eq "") {
    foreach ($cand in @("C:\Program Files\R\R-4.4.2\bin\Rscript.exe",
                        "C:\Program Files\R\R-4.4.1\bin\Rscript.exe")) {
        if (Test-Path $cand) { $Rscript = $cand; break }
    }
}
if ($Rscript -ne "" -and (Test-Path $Rscript)) {
    $env:R_HOME = Split-Path (Split-Path $Rscript)   # <R_HOME>\bin\Rscript.exe -> <R_HOME>
    Write-Host "R_HOME = $env:R_HOME"
} else {
    Write-Host "WARNING: Rscript not found; the engine may fail to start. Pass -Rscript <path>." -ForegroundColor Yellow
}

# --- pin JAVA_HOME to the Microsoft JDK 25 in the repo ----------------------
$jdk = Join-Path $repo "jdk-25.0.3+9"
if (-not (Test-Path (Join-Path $jdk "bin\java.exe"))) {
    throw "Microsoft JDK 25 not found at $jdk"
}
$env:JAVA_HOME = $jdk
$env:PATH = "$jdk\bin;$env:PATH"
Write-Host "JAVA_HOME = $env:JAVA_HOME"
& java -version

# --- engine: prefer the Python (FlowKit) engine if the venv is present -------
$pyExe = Join-Path $repo "engine\py-env\Scripts\python.exe"
if (Test-Path $pyExe) {
    $env:STREAMFLOW_ENGINE = "python"
    $env:STREAMFLOW_PYTHON = $pyExe
    $env:STREAMFLOW_ENGINE_PY = Join-Path $repo "engine\streamflow_engine.py"
    Write-Host "Engine = Python ($pyExe)"
} else {
    Write-Host "Engine = R (Python venv not found at $pyExe)"
}

# Trust the Windows certificate store so Maven Central works behind a
# TLS-intercepting proxy/AV (otherwise: PKIX 'unable to find valid cert path').
$env:MAVEN_OPTS = "-Djavax.net.ssl.trustStoreType=Windows-ROOT -Djavax.net.ssl.trustStore=NUL"

# Prefer the bundled Maven; fall back to one on PATH.
$mvn = Join-Path $repo "apache-maven-3.9.16\bin\mvn.cmd"
if (-not (Test-Path $mvn)) {
    if (Get-Command mvn -ErrorAction SilentlyContinue) { $mvn = "mvn" }
    else { throw "Maven not found (looked for $mvn and PATH)." }
}

# --- run --------------------------------------------------------------------
$mvnArgs = @("-B", "-f", (Join-Path $here "pom.xml"), "javafx:run")
if ($Rscript -ne "") {
    $mvnArgs += "-Dstreamflow.rscript=$Rscript"
}
Write-Host "Running: $mvn $($mvnArgs -join ' ')"
& $mvn @mvnArgs
