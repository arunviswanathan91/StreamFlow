'use strict';

const { app, BrowserWindow, Menu, dialog, ipcMain, shell } = require('electron');
const path = require('path');
const fs = require('fs');
const { spawn, exec } = require('child_process');
const http = require('http');
const log = require('electron-log');
const getPort = require('get-port');

// Configure logger
const logDir = path.join(app.getPath('appData'), 'StreamFLOW', 'logs');
if (!fs.existsSync(logDir)) {
  fs.mkdirSync(logDir, { recursive: true });
}
log.transports.file.resolvePathFn = () => path.join(logDir, 'streamflow.log');
log.transports.file.level = 'debug';

// Determine if running from packaged build or development
const isPackaged = app.isPackaged;

// In a packaged app there is no terminal attached to process.stdout/stderr, so
// electron-log's console transport throws EPIPE on every write. The crash handler
// then logs that error to the console, which throws EPIPE again — a self-feeding
// loop that floods the log. Disable the console transport when packaged; keep it
// in development where a real console exists.
log.transports.console.level = isPackaged ? false : 'debug';

let mainWindow = null;
let splashWindow = null;
let rProcess = null;
let shinyPort = null;
let pollInterval = null;
const resourcesPath = isPackaged
  ? process.resourcesPath
  : path.join(__dirname, '..');

const rScriptPath = path.join(resourcesPath, 'R-Portable', 'bin', 'Rscript.exe');
const shinyAppPath = path.join(resourcesPath, 'shiny', 'app.R');

function createSplashWindow() {
  splashWindow = new BrowserWindow({
    width: 600,
    height: 400,
    frame: false,
    transparent: false,
    alwaysOnTop: true,
    resizable: false,
    center: true,
    show: false,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false
    }
  });

  splashWindow.loadFile(path.join(__dirname, 'splash.html'));

  splashWindow.once('ready-to-show', () => {
    splashWindow.show();
    log.info('Splash screen displayed');
  });
}

function createMainWindow() {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 1100,
    minHeight: 700,
    frame: false,
    show: false,
    icon: path.join(__dirname, '..', 'build', 'icon.ico'),
    backgroundColor: '#0D1B2A',
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      preload: path.join(__dirname, 'preload.js'),
      webSecurity: true
    }
  });

  mainWindow.loadURL(`http://127.0.0.1:${shinyPort}`);

  mainWindow.once('ready-to-show', () => {
    if (splashWindow && !splashWindow.isDestroyed()) {
      splashWindow.close();
      splashWindow = null;
    }
    mainWindow.show();
    log.info('Main window displayed');
  });

  mainWindow.on('closed', () => {
    killRProcess();
    mainWindow = null;
  });

  buildAppMenu();
}

function buildAppMenu() {
  const template = [
    {
      label: 'File',
      submenu: [
        {
          label: 'Open FCS Folder',
          accelerator: 'CmdOrCtrl+O',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-open-fcs-folder');
            }
          }
        },
        {
          label: 'Save Session',
          accelerator: 'CmdOrCtrl+S',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-save-session');
            }
          }
        },
        {
          label: 'Export Results',
          accelerator: 'CmdOrCtrl+E',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-export-results');
            }
          }
        },
        { type: 'separator' },
        {
          label: 'Quit',
          accelerator: 'CmdOrCtrl+Q',
          click: () => {
            app.quit();
          }
        }
      ]
    },
    {
      label: 'View',
      submenu: [
        {
          label: 'Toggle Sidebar',
          accelerator: 'CmdOrCtrl+B',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-toggle-sidebar');
            }
          }
        },
        { type: 'separator' },
        {
          label: 'Zoom In',
          accelerator: 'CmdOrCtrl+=',
          click: () => {
            if (mainWindow) {
              const factor = mainWindow.webContents.getZoomFactor();
              mainWindow.webContents.setZoomFactor(Math.min(factor + 0.1, 3.0));
            }
          }
        },
        {
          label: 'Zoom Out',
          accelerator: 'CmdOrCtrl+-',
          click: () => {
            if (mainWindow) {
              const factor = mainWindow.webContents.getZoomFactor();
              mainWindow.webContents.setZoomFactor(Math.max(factor - 0.1, 0.3));
            }
          }
        },
        {
          label: 'Reset Zoom',
          accelerator: 'CmdOrCtrl+0',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.setZoomFactor(1.0);
            }
          }
        },
        { type: 'separator' },
        {
          label: 'Toggle DevTools',
          accelerator: 'F12',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.toggleDevTools();
            }
          }
        }
      ]
    },
    {
      label: 'Help',
      submenu: [
        {
          label: 'Documentation',
          click: () => {
            shell.openExternal('https://dillonhammill.github.io/CytoExploreR/');
          }
        },
        { type: 'separator' },
        {
          label: 'About StreamFLOW',
          click: () => {
            dialog.showMessageBox(mainWindow, {
              type: 'info',
              title: 'About StreamFLOW',
              message: 'StreamFLOW v1.0.0',
              detail: 'Flow Cytometry Analysis Suite\nPowered by CytoExploreR\n\nA professional desktop application for interactive flow cytometry data analysis.',
              buttons: ['OK'],
              icon: path.join(__dirname, '..', 'build', 'icon.ico')
            });
          }
        }
      ]
    }
  ];

  const menu = Menu.buildFromTemplate(template);
  Menu.setApplicationMenu(menu);
}

function startRProcess() {
  log.info(`Starting R process: ${rScriptPath}`);
  log.info(`Shiny app path: ${shinyAppPath}`);
  log.info(`Port: ${shinyPort}`);

  if (!fs.existsSync(rScriptPath)) {
    const errMsg = `R Portable not found at: ${rScriptPath}\n\nPlease run scripts/install.bat to set up R Portable.`;
    log.error(errMsg);
    dialog.showErrorBox('R Portable Not Found', errMsg);
    app.quit();
    return;
  }

  if (!fs.existsSync(shinyAppPath)) {
    const errMsg = `Shiny app not found at: ${shinyAppPath}`;
    log.error(errMsg);
    dialog.showErrorBox('App Files Missing', errMsg);
    app.quit();
    return;
  }

  rProcess = spawn(rScriptPath, [shinyAppPath, '--port', String(shinyPort)], {
    cwd: path.dirname(shinyAppPath),
    env: {
      ...process.env,
      R_HOME: path.join(resourcesPath, 'R-Portable'),
      R_LIBS_USER: path.join(resourcesPath, 'R-Portable', 'library')
    }
  });

  rProcess.stdout.on('data', (data) => {
    log.info(`[R] ${data.toString().trim()}`);
  });

  rProcess.stderr.on('data', (data) => {
    log.warn(`[R stderr] ${data.toString().trim()}`);
  });

  rProcess.on('error', (err) => {
    log.error(`Failed to start R process: ${err.message}`);
    dialog.showErrorBox('R Process Error', `Failed to start the analysis engine:\n\n${err.message}`);
    app.quit();
  });

  rProcess.on('exit', (code, signal) => {
    log.info(`R process exited with code ${code}, signal ${signal}`);
    if (code !== 0 && code !== null && mainWindow) {
      dialog.showErrorBox(
        'Analysis Engine Stopped',
        `The R analysis engine exited unexpectedly (code: ${code}).\n\nPlease check the log file at:\n${path.join(logDir, 'streamflow.log')}`
      );
    }
  });

  log.info('R process spawned, waiting for Shiny to be ready...');
}

function pollShinyReady() {
  let attempts = 0;
  const maxAttempts = 120;

  pollInterval = setInterval(() => {
    attempts++;
    if (attempts > maxAttempts) {
      clearInterval(pollInterval);
      log.error('Shiny failed to start within timeout');
      dialog.showErrorBox(
        'Startup Timeout',
        'The analysis engine failed to start within 60 seconds.\n\nPlease check the log file for details:\n' +
        path.join(logDir, 'streamflow.log')
      );
      app.quit();
      return;
    }

    const req = http.get(`http://127.0.0.1:${shinyPort}`, (res) => {
      if (res.statusCode === 200) {
        clearInterval(pollInterval);
        log.info(`Shiny is ready on port ${shinyPort} after ${attempts} attempts`);
        createMainWindow();
      }
    });

    req.on('error', () => {
      // Still waiting, not an error worth logging every 500ms
    });

    req.setTimeout(400, () => {
      req.destroy();
    });
  }, 500);
}

function killRProcess() {
  if (rProcess && !rProcess.killed) {
    log.info(`Killing R process (PID: ${rProcess.pid}) and children`);
    if (process.platform === 'win32') {
      exec(`taskkill /PID ${rProcess.pid} /T /F`, (err) => {
        if (err) {
          log.warn(`taskkill error: ${err.message}`);
        }
      });
    } else {
      rProcess.kill('SIGTERM');
    }
    rProcess = null;
  }
}

// IPC handlers for window controls (called from preload via renderer)
ipcMain.on('window-minimize', () => {
  if (mainWindow) mainWindow.minimize();
});

ipcMain.on('window-maximize', () => {
  if (!mainWindow) return;
  if (mainWindow.isMaximized()) {
    mainWindow.unmaximize();
  } else {
    mainWindow.maximize();
  }
});

ipcMain.on('window-close', () => {
  if (mainWindow) mainWindow.close();
});

ipcMain.handle('window-is-maximized', () => {
  return mainWindow ? mainWindow.isMaximized() : false;
});

// App lifecycle
app.whenReady().then(async () => {
  log.info('StreamFLOW starting up');
  log.info(`Resources path: ${resourcesPath}`);

  try {
    shinyPort = await getPort({ port: getPort.makeRange(3838, 4000) });
    log.info(`Selected port: ${shinyPort}`);
  } catch (err) {
    log.error(`Failed to get port: ${err.message}`);
    shinyPort = 3838;
  }

  createSplashWindow();
  startRProcess();
  pollShinyReady();
});

app.on('window-all-closed', () => {
  killRProcess();
  app.quit();
});

app.on('before-quit', () => {
  if (pollInterval) {
    clearInterval(pollInterval);
  }
  killRProcess();
});

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0 && shinyPort) {
    createMainWindow();
  }
});

// The console transport is disabled when packaged (see logger config above), so
// log.error here can only reach the file transport and cannot re-enter the EPIPE
// loop. The try/catch is a final guard so the crash handler itself never throws.
function logFatal(label, err) {
  const detail = err && err.stack ? err.stack : String(err);
  try {
    log.error(`${label}: ${detail}`);
  } catch (_) {
    // Never let the crash handler itself throw.
  }
}

process.on('uncaughtException', (err) => {
  logFatal('Uncaught exception', err);
});

process.on('unhandledRejection', (reason) => {
  logFatal('Unhandled promise rejection', reason);
});
