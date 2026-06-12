'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('electronAPI', {
  // Window controls
  minimize: () => ipcRenderer.send('window-minimize'),
  maximize: () => ipcRenderer.send('window-maximize'),
  close: () => ipcRenderer.send('window-close'),
  isMaximized: () => ipcRenderer.invoke('window-is-maximized'),

  // Open a focused single-sample pop-out window (FlowJo-style)
  openSampleWindow: (opts) => ipcRenderer.send('open-sample-window', opts),

  // Native file/folder pickers (replace the broken shinyFiles modal).
  // Each resolves to { canceled: boolean, path: string | null }.
  selectFolder: (options) => ipcRenderer.invoke('dialog:selectFolder', options),
  selectFile:   (options) => ipcRenderer.invoke('dialog:selectFile', options),
  saveFile:     (options) => ipcRenderer.invoke('dialog:saveFile', options),

  // Menu event listeners (Shiny listens for these via JS)
  onOpenFcsFolder: (callback) => ipcRenderer.on('menu-open-fcs-folder', callback),
  onSaveSession: (callback) => ipcRenderer.on('menu-save-session', callback),
  onOpenWorkspace: (callback) => ipcRenderer.on('menu-open-workspace', callback),
  onExportResults: (callback) => ipcRenderer.on('menu-export-results', callback),
  onToggleSidebar: (callback) => ipcRenderer.on('menu-toggle-sidebar', callback),

  // Remove listeners
  removeAllListeners: (channel) => ipcRenderer.removeAllListeners(channel),

  // Platform info
  platform: process.platform,
  appVersion: process.env.npm_package_version || '1.0.0'
});
