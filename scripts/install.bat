@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul 2>&1
title StreamFLOW Installer

:: ============================================================
::  StreamFLOW - Dependency Installer
::  Run this once before first launch.
:: ============================================================

echo.
echo  ╔══════════════════════════════════════════════════════╗
echo  ║                                                      ║
echo  ║   ███████╗████████╗██████╗ ███████╗ █████╗ ███╗   ███╗║
echo  ║   ██╔════╝╚══██╔══╝██╔══██╗██╔════╝██╔══██╗████╗ ████║║
echo  ║   ███████╗   ██║   ██████╔╝█████╗  ███████║██╔████╔██║║
echo  ║   ╚════██║   ██║   ██╔══██╗██╔══╝  ██╔══██║██║╚██╔╝██║║
echo  ║   ███████║   ██║   ██║  ██║███████╗██║  ██║██║ ╚═╝ ██║║
echo  ║   ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝║
echo  ║              FLOW                                    ║
echo  ║        Flow Cytometry Analysis Suite                 ║
echo  ║                     v1.0.0                           ║
echo  ╚══════════════════════════════════════════════════════╝
echo.

:: Determine script directory
set "SCRIPT_DIR=%~dp0"
set "ROOT_DIR=%SCRIPT_DIR%.."
set "R_PORTABLE_DIR=%ROOT_DIR%\R-Portable"
set "RSCRIPT=%R_PORTABLE_DIR%\bin\Rscript.exe"
set "INSTALL_SCRIPT=%SCRIPT_DIR%install_packages.R"
set "LOG_FILE=%ROOT_DIR%\install_log.txt"

echo  [1/4] Checking R Portable...
echo.

if not exist "%R_PORTABLE_DIR%" (
    echo  ╔══════════════════════════════════════════════════════╗
    echo  ║  R PORTABLE NOT FOUND                                ║
    echo  ╚══════════════════════════════════════════════════════╝
    echo.
    echo  R Portable was not found at:
    echo    %R_PORTABLE_DIR%
    echo.
    echo  To set up StreamFLOW, please:
    echo.
    echo  1. Download R Portable 4.3.x from:
    echo     https://sourceforge.net/projects/rportable/
    echo.
    echo  2. Extract it so the folder structure looks like:
    echo     StreamFLOW\
    echo       R-Portable\
    echo         bin\
    echo           Rscript.exe
    echo           R.exe
    echo         library\
    echo           ...
    echo.
    echo  3. Re-run this installer.
    echo.
    pause
    exit /b 1
)

if not exist "%RSCRIPT%" (
    echo  ERROR: Rscript.exe not found at:
    echo    %RSCRIPT%
    echo.
    echo  The R-Portable folder exists but appears incomplete.
    echo  Please re-download R Portable from:
    echo    https://sourceforge.net/projects/rportable/
    echo.
    pause
    exit /b 1
)

echo  R Portable found at: %R_PORTABLE_DIR%
echo.
echo  [2/4] Checking install script...

if not exist "%INSTALL_SCRIPT%" (
    echo  ERROR: install_packages.R not found at:
    echo    %INSTALL_SCRIPT%
    echo.
    pause
    exit /b 1
)

echo  Install script found.
echo.
echo  [3/4] Installing R packages (this may take 10-30 minutes)...
echo        All output is logged to: %LOG_FILE%
echo.

:: Write header to log
echo StreamFLOW Package Installation > "%LOG_FILE%"
echo Started: %DATE% %TIME% >> "%LOG_FILE%"
echo. >> "%LOG_FILE%"

:: Run installer with R Portable
set "R_HOME=%R_PORTABLE_DIR%"
set "R_LIBS_USER=%R_PORTABLE_DIR%\library"

"%RSCRIPT%" --vanilla "%INSTALL_SCRIPT%" >> "%LOG_FILE%" 2>&1

if %ERRORLEVEL% NEQ 0 (
    echo.
    echo  ╔══════════════════════════════════════════════════════╗
    echo  ║  INSTALLATION ENCOUNTERED ERRORS                     ║
    echo  ╚══════════════════════════════════════════════════════╝
    echo.
    echo  Some packages may have failed to install.
    echo  Please check the log file for details:
    echo    %LOG_FILE%
    echo.
    echo  Common fixes:
    echo    - Ensure internet connection is active
    echo    - Run as Administrator if permission errors occur
    echo    - Check CRAN is accessible from your network
    echo.
) else (
    echo.
    echo  ╔══════════════════════════════════════════════════════╗
    echo  ║  INSTALLATION COMPLETE                               ║
    echo  ╚══════════════════════════════════════════════════════╝
    echo.
    echo  All packages installed successfully!
    echo  Log saved to: %LOG_FILE%
    echo.
)

echo  [4/4] Done!
echo.
echo  To launch StreamFLOW:
echo    - Run the StreamFLOW desktop shortcut, OR
echo    - Run: scripts\launch.bat  (fallback browser mode)
echo.

set /p "LAUNCH=Launch StreamFLOW now? (Y/N): "
if /i "%LAUNCH%"=="Y" (
    echo.
    echo  Starting StreamFLOW...
    cd "%ROOT_DIR%"
    call scripts\launch.bat
)

endlocal
