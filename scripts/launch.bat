@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul 2>&1
title StreamFLOW - Launching...

:: ============================================================
::  StreamFLOW - Fallback Launcher (Browser Mode)
::  Use this if Electron is not available.
:: ============================================================

set "SCRIPT_DIR=%~dp0"
set "ROOT_DIR=%SCRIPT_DIR%.."
set "R_PORTABLE_DIR=%ROOT_DIR%\R-Portable"
set "RSCRIPT=%R_PORTABLE_DIR%\bin\Rscript.exe"
set "APP_R=%ROOT_DIR%\shiny\app.R"
set "PORT=3838"

echo.
echo  StreamFLOW  ^|  Flow Cytometry Analysis Suite
echo  ------------------------------------------------
echo.

:: Check R Portable
if not exist "%RSCRIPT%" (
    echo  ERROR: R Portable not found.
    echo  Please run scripts\install.bat first.
    echo.
    pause
    exit /b 1
)

:: Check app files
if not exist "%APP_R%" (
    echo  ERROR: Shiny app not found at:
    echo    %APP_R%
    echo.
    pause
    exit /b 1
)

:: Find a free port (try 3838, then increment)
:find_port
netstat -an | find ":%PORT% " >nul 2>&1
if %ERRORLEVEL%==0 (
    set /a PORT+=1
    if %PORT% GTR 3900 (
        echo  ERROR: No free port found between 3838 and 3900.
        pause
        exit /b 1
    )
    goto find_port
)

echo  Starting R analysis engine on port %PORT%...
echo  R Portable: %R_PORTABLE_DIR%
echo.

set "R_HOME=%R_PORTABLE_DIR%"
set "R_LIBS_USER=%R_PORTABLE_DIR%\library"

:: Start R Shiny in a new background window
start "StreamFLOW-R" /min "%RSCRIPT%" "%APP_R%" --port %PORT%

:: Wait 5 seconds for Shiny to start
echo  Waiting for analysis engine to start...
timeout /t 5 /nobreak >nul

:: Check if Shiny is responding
:wait_shiny
powershell -Command "try { $r=(Invoke-WebRequest -Uri 'http://127.0.0.1:%PORT%' -TimeoutSec 2 -UseBasicParsing).StatusCode; if($r -eq 200){exit 0}else{exit 1} } catch { exit 1 }" >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo  Still waiting...
    timeout /t 3 /nobreak >nul
    goto wait_shiny
)

echo.
echo  StreamFLOW is ready!
echo  Opening browser at: http://127.0.0.1:%PORT%
echo.

:: Open browser
start "" "http://127.0.0.1:%PORT%"

echo  ╔══════════════════════════════════════════════╗
echo  ║  StreamFLOW is running in your browser       ║
echo  ║  URL: http://127.0.0.1:%PORT%                   ║
echo  ║                                              ║
echo  ║  Keep this window open while using the app. ║
echo  ║  Close this window to stop StreamFLOW.      ║
echo  ╚══════════════════════════════════════════════╝
echo.
echo  Press Ctrl+C or close this window to stop the server.
echo.

:: Keep window open and wait for user to close
:keepalive
timeout /t 60 /nobreak >nul
powershell -Command "try { $r=(Invoke-WebRequest -Uri 'http://127.0.0.1:%PORT%' -TimeoutSec 2 -UseBasicParsing).StatusCode; if($r -ne 200){exit 1} } catch { exit 1 }" >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo  Analysis engine stopped.
    goto cleanup
)
goto keepalive

:cleanup
echo  StreamFLOW has stopped.
pause
endlocal
