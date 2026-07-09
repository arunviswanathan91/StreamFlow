# StreamFLOW — build a self-contained Windows app via jpackage.
#
# Produces an app-image (folder with StreamFLOW.exe + bundled JRE) that includes
# the JavaFX app, BOTH engines' scripts, and two staged runtimes — a portable
# Python (FlowKit, the default engine) and a portable R (for R-based plugins) — so
# the end user needs no Java, Python, or R installed ("fat binary", ~1.5 GB).
#
#   1) when Python deps change:  powershell -File engine\stage-python.ps1
#   2) when R deps change:       powershell -File engine\stage-r-portable.ps1
#   3) build the app:            powershell -File app\package.ps1
#       [-Installer]   also build an .exe/.msi installer (needs WiX on PATH)
#
# NOTE: first run needs validation (jpackage classpath + runtime path resolution).
#
#   -Installer        build a Windows .exe installer (needs WiX 3.x on PATH) instead of an app-image
#   -SkipR            build a Python-only fat binary (omit the bundled R runtime / R plugins)
#   -Version <x.y.z>  app version stamped into the package (default 1.2.0)
#
# CI-friendly: if the repo-bundled JDK/Maven are absent it falls back to JAVA_HOME / the `mvn`
# on PATH, and R is skipped automatically when R-Portable has not been staged.
param([switch]$Installer, [switch]$SkipR, [string]$Version = "1.2.0")

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path     # app\
$repo = Split-Path -Parent $here

# Prefer the repo-bundled JDK (local dev); fall back to JAVA_HOME (CI). Either must carry jpackage.
$jdk = Join-Path $repo "jdk-25.0.3+9"
if (-not (Test-Path (Join-Path $jdk "bin\jpackage.exe"))) {
    if ($env:JAVA_HOME -and (Test-Path (Join-Path $env:JAVA_HOME "bin\jpackage.exe"))) {
        $jdk = $env:JAVA_HOME
    } else {
        throw "No JDK with jpackage found. Set JAVA_HOME to a JDK 21+ (with jpackage), or place jdk-25.0.3+9 in the repo."
    }
}
# Prefer the repo-bundled Maven; fall back to `mvn` on PATH (CI / system install).
$mvn = Join-Path $repo "apache-maven-3.9.16\bin\mvn.cmd"
if (-not (Test-Path $mvn)) { $mvn = "mvn" }

$env:JAVA_HOME = $jdk
$env:PATH = "$jdk\bin;$env:PATH"
$env:MAVEN_OPTS = "-Djavax.net.ssl.trustStoreType=Windows-ROOT -Djavax.net.ssl.trustStore=NUL"

# R is optional: bundle it only when R-Portable has been staged and -SkipR was not passed.
$rportable = Join-Path $here "target\R-Portable"
$bundleR = (-not $SkipR) -and (Test-Path (Join-Path $rportable "bin\Rscript.exe"))
if (-not $bundleR) {
    Write-Host "R-Portable not bundled (-SkipR or not staged) -> Python-only fat binary; R plugins unavailable."
}

$pyportable = Join-Path $here "target\python"
if (-not (Test-Path (Join-Path $pyportable "python.exe"))) {
    throw "Portable Python not staged. Run: powershell -File engine\stage-python.ps1"
}

# 1) Build the jar + collect runtime dependencies into target\libs.
# NOTE: deliberately NOT `mvn clean` — target\python (stage-python.ps1) and target\R-Portable
# (the release workflow) are staged INTO target before this runs, and `clean` would delete them,
# producing an installer with no engine. Remove only the Maven build outputs instead.
foreach ($d in @("classes", "test-classes", "libs", "content", "dist",
                 "generated-sources", "maven-status", "surefire-reports")) {
    $p = Join-Path $here "target\$d"
    if (Test-Path $p) { Remove-Item -Recurse -Force $p }
}
Get-ChildItem (Join-Path $here "target") -Filter "streamflow-*.jar" -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue

Write-Host "Building jar + collecting dependencies…"
& $mvn -B -f (Join-Path $here "pom.xml") package `
    dependency:copy-dependencies "-DoutputDirectory=target/libs" "-DincludeScope=runtime" "-DskipTests=true"
if ($LASTEXITCODE -ne 0) { throw "Maven build failed (exit $LASTEXITCODE)." }
# Resolve the built jar by pattern so the script isn't tied to a hard-coded version string.
$jar = Get-ChildItem (Join-Path $here "target") -Filter "streamflow-*.jar" | Select-Object -First 1
if (-not $jar) { throw "Built jar not found in target\." }
$jarName = $jar.Name
Copy-Item $jar.FullName (Join-Path $here "target\libs\") -Force

# 2) Stage app content (engine scripts + portable Python + R-Portable) next to the launcher.
#    Excludes engine/vendor (R package SOURCE — only needed at R-Portable stage time)
#    and engine/py-env (the dev venv — the relocatable target\python is bundled instead).
$content = Join-Path $here "target\content"
if (Test-Path $content) { Remove-Item -Recurse -Force $content }
New-Item -ItemType Directory -Force -Path $content | Out-Null
robocopy (Join-Path $repo "engine") (Join-Path $content "engine") /E /XD "vendor" "py-env" "__pycache__" /NFL /NDL /NJH /NJS | Out-Null
robocopy $pyportable (Join-Path $content "python") /E /NFL /NDL /NJH /NJS | Out-Null
if ($bundleR) { robocopy $rportable (Join-Path $content "R-Portable") /E /NFL /NDL /NJH /NJS | Out-Null }

# 3) jpackage app-image (JavaFX runs from the classpath; the openjfx win jars carry natives).
$dist = Join-Path $here "target\dist"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
$type = if ($Installer) { "exe" } else { "app-image" }
Write-Host "Running jpackage ($type)…"
& (Join-Path $jdk "bin\jpackage.exe") `
    --type $type `
    --name "StreamFLOW" `
    --app-version $Version `
    --input (Join-Path $here "target\libs") `
    --main-jar $jarName `
    --main-class "org.streamflow.StreamFlowApp" `
    --app-content $content `
    --icon (Join-Path $repo "build\icon.ico") `
    --dest $dist `
    --java-options "-Dstreamflow.appDir=`$APPDIR\content" `
    --java-options "-Dstreamflow.engine.kind=python" `
    --vendor "StreamFLOW"
if ($LASTEXITCODE -ne 0) { throw "jpackage failed (exit $LASTEXITCODE)." }

Write-Host "Built -> $dist"
Get-ChildItem $dist | ForEach-Object { Write-Host ("  {0}" -f $_.Name) }
