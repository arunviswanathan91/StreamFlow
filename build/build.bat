@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul 2>&1
title StreamFLOW - Build

:: ============================================================
::  StreamFLOW - Electron Builder
::  Produces a Windows NSIS installer in dist/
:: ============================================================

set "SCRIPT_DIR=%~dp0"
set "ROOT_DIR=%SCRIPT_DIR%.."

echo.
echo  ╔══════════════════════════════════════════════════╗
echo  ║  StreamFLOW Build System  ^|  v1.0.0             ║
echo  ╚══════════════════════════════════════════════════╝
echo.

cd /d "%ROOT_DIR%"

:: Check Node.js
where node >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo  ERROR: Node.js is not installed or not in PATH.
    echo  Download Node.js from: https://nodejs.org/
    pause
    exit /b 1
)

for /f "tokens=*" %%v in ('node --version') do set NODE_VER=%%v
echo  Node.js: %NODE_VER%

:: Check npm
where npm >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo  ERROR: npm not found.
    pause
    exit /b 1
)

for /f "tokens=*" %%v in ('npm --version') do set NPM_VER=%%v
echo  npm: %NPM_VER%
echo.

:: Check R-Portable is present before building
if not exist "%ROOT_DIR%\R-Portable" (
    echo  WARNING: R-Portable folder not found at:
    echo    %ROOT_DIR%\R-Portable
    echo.
    echo  The installer will be built without R-Portable.
    echo  Users will need to run install.bat after installation.
    echo.
    set /p "CONTINUE=Continue build anyway? (Y/N): "
    if /i not "!CONTINUE!"=="Y" (
        echo  Build cancelled.
        pause
        exit /b 0
    )
)

:: Step 1: Install npm dependencies
echo  [1/3] Installing npm dependencies...
echo.
call npm install
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo  ERROR: npm install failed. Check the output above.
    pause
    exit /b 1
)
echo.
echo  Dependencies installed.
echo.

:: Step 2: Create placeholder icon if not present
if not exist "%ROOT_DIR%\build\icon.ico" (
    echo  NOTE: build\icon.ico not found.
    echo  Using a placeholder. Replace with your own 256x256 .ico file.
    echo.
    :: Create a minimal valid ICO file using PowerShell
    powershell -Command ^
        "$iconPath='%ROOT_DIR%\build\icon.ico'; " ^
        "Add-Type -AssemblyName System.Drawing; " ^
        "$bmp=New-Object System.Drawing.Bitmap(256,256); " ^
        "$g=[System.Drawing.Graphics]::FromImage($bmp); " ^
        "$g.Clear([System.Drawing.Color]::FromArgb(255,13,27,42)); " ^
        "$b=New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255,0,180,216)); " ^
        "$g.FillEllipse($b,28,28,200,200); " ^
        "$icon=[System.Drawing.Icon]::FromHandle($bmp.GetHicon()); " ^
        "$fs=[System.IO.File]::Open($iconPath,[System.IO.FileMode]::Create); " ^
        "$icon.Save($fs); $fs.Close();" >nul 2>&1
    if exist "%ROOT_DIR%\build\icon.ico" (
        echo  Placeholder icon created.
    ) else (
        echo  Could not create placeholder icon. Build may warn about missing icon.
    )
    echo.
)

:: Step 3: Run electron-builder
echo  [2/3] Building StreamFLOW installer...
echo        This may take several minutes depending on R-Portable size.
echo.

call npm run build
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo  ERROR: electron-builder failed. Check the output above.
    echo.
    echo  Common fixes:
    echo    - Ensure build\icon.ico exists (256x256 pixels)
    echo    - Check that R-Portable is present if extraResources are configured
    echo    - Run as Administrator for code signing
    echo.
    pause
    exit /b 1
)

:: Step 4: Report output
echo.
echo  [3/3] Build complete!
echo.

if exist "%ROOT_DIR%\dist\" (
    echo  Output files in dist\:
    dir "%ROOT_DIR%\dist\" /b /a:-d 2>nul | findstr /i "\.exe$" && (
        echo.
        for %%f in ("%ROOT_DIR%\dist\*.exe") do (
            echo  Installer: %%f
            echo  Size:      %%~zf bytes
        )
    )
) else (
    echo  dist\ folder not found — check electron-builder output above.
)

echo.
echo  ╔══════════════════════════════════════════════════╗
echo  ║  StreamFLOW-Setup.exe is ready to distribute.   ║
echo  ╚══════════════════════════════════════════════════╝
echo.

pause
endlocal
