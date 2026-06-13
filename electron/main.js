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
// Cap the log so a long session piping verbose R stdout/stderr can't grow it
// unbounded; electron-log rotates to streamflow.old.log past this size.
log.transports.file.maxSize = 10 * 1024 * 1024; // 10 MB

// Official download location, surfaced in the runtime error if the bundled R
// runtime is ever missing (e.g. a user deleted resources/R-Portable). Update
// this if the repository moves.
const GITHUB_RELEASES_URL = 'https://github.com/arunviswanathan91/StreamFlow/releases';

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
const popoutWindows = new Set();
const resourcesPath = isPackaged
  ? process.resourcesPath
  : path.join(__dirname, '..');

const rScriptPath = path.join(resourcesPath, 'R-Portable', 'bin', 'Rscript.exe');
const shinyAppPath = path.join(resourcesPath, 'shiny', 'app.R');

// Lock a window down: it may only navigate within the local Shiny server, and
// any attempt to open a new window (target=_blank, window.open, etc.) is denied.
// External http/https links are routed out to the user's real browser instead of
// hijacking the app window. This guards against a malicious or unexpected link
// rendered from FCS-metadata-derived strings inside the Shiny UI.
function isAllowedAppUrl(targetUrl) {
  if (!shinyPort) return false;
  try {
    const u = new URL(targetUrl);
    return (u.protocol === 'http:' || u.protocol === 'https:') &&
           u.hostname === '127.0.0.1' &&
           u.port === String(shinyPort);
  } catch (_) {
    return false;
  }
}

function hardenWindow(win) {
  win.webContents.on('will-navigate', (event, targetUrl) => {
    if (!isAllowedAppUrl(targetUrl)) {
      event.preventDefault();
      log.warn(`Blocked in-app navigation to: ${targetUrl}`);
    }
  });

  win.webContents.setWindowOpenHandler(({ url }) => {
    try {
      const u = new URL(url);
      if (u.protocol === 'http:' || u.protocol === 'https:') {
        shell.openExternal(url);
      } else {
        log.warn(`Blocked window open with non-web protocol: ${url}`);
      }
    } catch (_) {
      log.warn(`Blocked window open with invalid url: ${url}`);
    }
    return { action: 'deny' };
  });
}

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
  hardenWindow(splashWindow);

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
  hardenWindow(mainWindow);

  mainWindow.once('ready-to-show', () => {
    if (splashWindow && !splashWindow.isDestroyed()) {
      splashWindow.close();
      splashWindow = null;
    }
    mainWindow.show();
    log.info('Main window displayed');
  });

  mainWindow.on('closed', () => {
    closeAllPopouts();
    killRProcess();
    mainWindow = null;
  });

  buildAppMenu();
}

// FlowJo-style pop-out: a focused single-sample window backed by the same Shiny
// server (?view=popout). Closing one does not stop R — only the main window does.
function openSampleWindow(opts) {
  if (!shinyPort) {
    log.warn('Pop-out requested before Shiny port was ready');
    return;
  }
  const o = opts || {};
  const params = new URLSearchParams({
    view: 'popout',
    sample: o.sample || '',
    x: o.x || '',
    y: o.y || '',
    dim: o.dim || '2d'
  });

  const win = new BrowserWindow({
    width: 940,
    height: 760,
    backgroundColor: '#0D1B2A',
    icon: path.join(__dirname, '..', 'build', 'icon.ico'),
    title: `StreamFLOW — ${o.sample || 'Sample'}`,
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      preload: path.join(__dirname, 'preload.js'),
      webSecurity: true
    }
  });

  win.setMenuBarVisibility(false);
  win.loadURL(`http://127.0.0.1:${shinyPort}/?${params.toString()}`);
  hardenWindow(win);
  popoutWindows.add(win);
  win.on('closed', () => popoutWindows.delete(win));
  log.info(`Opened pop-out window for sample: ${o.sample}`);
}

function closeAllPopouts() {
  for (const win of popoutWindows) {
    if (!win.isDestroyed()) win.close();
  }
  popoutWindows.clear();
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
          label: 'Save Workspace…',
          accelerator: 'CmdOrCtrl+S',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-save-session');
            }
          }
        },
        {
          label: 'Open Workspace…',
          accelerator: 'CmdOrCtrl+Shift+O',
          click: () => {
            if (mainWindow) {
              mainWindow.webContents.send('menu-open-workspace');
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
    // Dump what *is* present under resourcesPath so streamflow.log turns any
    // future occurrence of this into a 5-second diagnosis instead of a repeat
    // debugging session.
    let listing = '(could not read resources directory)';
    try {
      listing = fs.readdirSync(resourcesPath).join(', ');
    } catch (e) {
      listing = `(readdir failed: ${e.message})`;
    }
    log.error(`R Portable not found at: ${rScriptPath}`);
    log.error(`Contents of resourcesPath (${resourcesPath}): ${listing}`);

    const errMsg =
      'This installer is missing the bundled R runtime.\n\n' +
      'Please reinstall using the full StreamFLOW installer from the official ' +
      `release page:\n${GITHUB_RELEASES_URL}\n\n` +
      'If reinstalling does not help, contact support.\n\n' +
      `Resources path: ${resourcesPath}`;
    dialog.showErrorBox('R Runtime Missing', errMsg);
    app.quit();
    return false;
  }

  if (!fs.existsSync(shinyAppPath)) {
    const errMsg = `Shiny app not found at: ${shinyAppPath}`;
    log.error(errMsg);
    dialog.showErrorBox('App Files Missing', errMsg);
    app.quit();
    return false;
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
  return true;
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

ipcMain.on('open-sample-window', (_evt, opts) => {
  openSampleWindow(opts);
});

// ── Native file/folder dialogs ───────────────────────────────────────────────
// These replace the shinyFiles browser modal (broken inside Chromium). The
// renderer invokes them via the preload bridge; each returns { canceled, path }
// and never throws across the IPC boundary.
function dialogParent() {
  return (mainWindow && !mainWindow.isDestroyed()) ? mainWindow : null;
}

ipcMain.handle('dialog:selectFolder', async (_evt, options) => {
  const parent = dialogParent();
  if (!parent) return { canceled: true, path: null };
  const opts = options || {};
  try {
    const result = await dialog.showOpenDialog(parent, {
      title: opts.title || 'Select folder',
      defaultPath: opts.defaultPath || undefined,
      properties: ['openDirectory']
    });
    if (result.canceled || !result.filePaths || result.filePaths.length === 0) {
      log.info('selectFolder: canceled');
      return { canceled: true, path: null };
    }
    log.info(`selectFolder: ${result.filePaths[0]}`);
    return { canceled: false, path: result.filePaths[0] };
  } catch (err) {
    log.error(`selectFolder failed: ${err.message}`);
    return { canceled: true, path: null };
  }
});

ipcMain.handle('dialog:selectFile', async (_evt, options) => {
  const parent = dialogParent();
  if (!parent) return { canceled: true, path: null };
  const opts = options || {};
  try {
    const result = await dialog.showOpenDialog(parent, {
      title: opts.title || 'Select file',
      defaultPath: opts.defaultPath || undefined,
      filters: Array.isArray(opts.filters) ? opts.filters : undefined,
      properties: ['openFile']
    });
    if (result.canceled || !result.filePaths || result.filePaths.length === 0) {
      log.info('selectFile: canceled');
      return { canceled: true, path: null };
    }
    log.info(`selectFile: ${result.filePaths[0]}`);
    return { canceled: false, path: result.filePaths[0] };
  } catch (err) {
    log.error(`selectFile failed: ${err.message}`);
    return { canceled: true, path: null };
  }
});

ipcMain.handle('dialog:selectFiles', async (_evt, options) => {
  const parent = dialogParent();
  if (!parent) return { canceled: true, paths: null };
  const opts = options || {};
  try {
    const result = await dialog.showOpenDialog(parent, {
      title: opts.title || 'Select files',
      defaultPath: opts.defaultPath || undefined,
      filters: Array.isArray(opts.filters) ? opts.filters : undefined,
      properties: ['openFile', 'multiSelections']
    });
    if (result.canceled || !result.filePaths || result.filePaths.length === 0) {
      log.info('selectFiles: canceled');
      return { canceled: true, paths: null };
    }
    log.info(`selectFiles: ${result.filePaths.length} file(s) selected`);
    return { canceled: false, paths: result.filePaths };
  } catch (err) {
    log.error(`selectFiles failed: ${err.message}`);
    return { canceled: true, paths: null };
  }
});

ipcMain.handle('dialog:saveFile', async (_evt, options) => {
  const parent = dialogParent();
  if (!parent) return { canceled: true, path: null };
  const opts = options || {};
  try {
    const result = await dialog.showSaveDialog(parent, {
      title: opts.title || 'Save file',
      defaultPath: opts.defaultPath || undefined,
      filters: Array.isArray(opts.filters) ? opts.filters : undefined
    });
    if (result.canceled || !result.filePath) {
      log.info('saveFile: canceled');
      return { canceled: true, path: null };
    }
    log.info(`saveFile: ${result.filePath}`);
    return { canceled: false, path: result.filePath };
  } catch (err) {
    log.error(`saveFile failed: ${err.message}`);
    return { canceled: true, path: null };
  }
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
  // Only start polling if R actually launched. The missing-R / missing-app
  // branches call app.quit() and return false; without this guard pollShinyReady
  // would still set an interval polling a port that will never bind.
  if (startRProcess()) {
    pollShinyReady();
  }
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
