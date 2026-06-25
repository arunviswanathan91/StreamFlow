# StreamFLOW (Java) — build & test
# Pins JAVA_HOME to the bundled Microsoft JDK 25 (LTS), then compiles and runs
# the protocol conformance test (Phase 0 verification gate).
#
# Usage:
#   ./build.ps1                          # compile + test (test skips if R absent)
#   ./build.ps1 -Rscript "C:\R\bin\Rscript.exe"   # run the conformance test against R
#   ./build.ps1 -SkipTests
#
param(
    [string]$Rscript = "",
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $here

$jdk = Join-Path $repo "jdk-25.0.3+9"
if (-not (Test-Path (Join-Path $jdk "bin\java.exe"))) {
    throw "Microsoft JDK 25 not found at $jdk"
}
$env:JAVA_HOME = $jdk
$env:PATH = "$jdk\bin;$env:PATH"
Write-Host "JAVA_HOME = $env:JAVA_HOME"

# Trust the Windows certificate store so Maven Central works behind a
# TLS-intercepting proxy/AV (otherwise: PKIX 'unable to find valid cert path').
$env:MAVEN_OPTS = "-Djavax.net.ssl.trustStoreType=Windows-ROOT -Djavax.net.ssl.trustStore=NUL"

# Prefer the bundled Maven; fall back to one on PATH.
$mvn = Join-Path $repo "apache-maven-3.9.16\bin\mvn.cmd"
if (-not (Test-Path $mvn)) {
    if (Get-Command mvn -ErrorAction SilentlyContinue) { $mvn = "mvn" }
    else { throw "Maven not found (looked for $mvn and PATH)." }
}

$goal = if ($SkipTests) { "compile" } else { "test" }
$mvnArgs = @("-B", "-f", (Join-Path $here "pom.xml"), "clean", $goal)
if ($Rscript -ne "") {
    $mvnArgs += "-Dstreamflow.rscript=$Rscript"
}
Write-Host "Running: $mvn $($mvnArgs -join ' ')"
& $mvn @mvnArgs
