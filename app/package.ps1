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
param([switch]$Installer)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path     # app\
$repo = Split-Path -Parent $here
$jdk  = Join-Path $repo "jdk-25.0.3+9"
$mvn  = Join-Path $repo "apache-maven-3.9.16\bin\mvn.cmd"
$env:JAVA_HOME = $jdk
$env:PATH = "$jdk\bin;$env:PATH"
$env:MAVEN_OPTS = "-Djavax.net.ssl.trustStoreType=Windows-ROOT -Djavax.net.ssl.trustStore=NUL"

$rportable = Join-Path $here "target\R-Portable"
if (-not (Test-Path (Join-Path $rportable "bin\Rscript.exe"))) {
    throw "R-Portable not staged. Run: powershell -File engine\stage-r-portable.ps1"
}

$pyportable = Join-Path $here "target\python"
if (-not (Test-Path (Join-Path $pyportable "python.exe"))) {
    throw "Portable Python not staged. Run: powershell -File engine\stage-python.ps1"
}

# 1) Build the jar + collect runtime dependencies into target\libs.
Write-Host "Building jar + collecting dependencies…"
& $mvn -B -f (Join-Path $here "pom.xml") clean package `
    dependency:copy-dependencies "-DoutputDirectory=target/libs" "-DincludeScope=runtime" "-DskipTests=true"
Copy-Item (Join-Path $here "target\streamflow-2.0.0.jar") (Join-Path $here "target\libs\") -Force

# 2) Stage app content (engine scripts + portable Python + R-Portable) next to the launcher.
#    Excludes engine/vendor (R package SOURCE — only needed at R-Portable stage time)
#    and engine/py-env (the dev venv — the relocatable target\python is bundled instead).
$content = Join-Path $here "target\content"
if (Test-Path $content) { Remove-Item -Recurse -Force $content }
New-Item -ItemType Directory -Force -Path $content | Out-Null
robocopy (Join-Path $repo "engine") (Join-Path $content "engine") /E /XD "vendor" "py-env" "__pycache__" /NFL /NDL /NJH /NJS | Out-Null
robocopy $pyportable (Join-Path $content "python") /E /NFL /NDL /NJH /NJS | Out-Null
robocopy $rportable (Join-Path $content "R-Portable") /E /NFL /NDL /NJH /NJS | Out-Null

# 3) jpackage app-image (JavaFX runs from the classpath; the openjfx win jars carry natives).
$dist = Join-Path $here "target\dist"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
$type = if ($Installer) { "exe" } else { "app-image" }
Write-Host "Running jpackage ($type)…"
& (Join-Path $jdk "bin\jpackage.exe") `
    --type $type `
    --name "StreamFLOW" `
    --app-version "2.0.0" `
    --input (Join-Path $here "target\libs") `
    --main-jar "streamflow-2.0.0.jar" `
    --main-class "org.streamflow.StreamFlowApp" `
    --app-content $content `
    --icon (Join-Path $repo "build\icon.ico") `
    --dest $dist `
    --java-options "-Dstreamflow.appDir=`$APPDIR\..\content" `
    --java-options "-Dstreamflow.engine.kind=python" `
    --vendor "StreamFLOW"

Write-Host "Built -> $dist"
